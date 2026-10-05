//! Upload targets: a plugin function that sends a finished export somewhere.
//!
//! The script gets `tempo.http` and `tempo.json` on top of the usual sandbox.
//! Requests are made by the `curl` program, only over HTTPS, and only to the
//! hosts the plugin lists under `[permissions] network`. The script never sees
//! the file's path; it can only ask for the file to be sent.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Instant;

use mlua::{Lua, Table, Value};

use crate::{first_line, sandbox, script_error, Plugin, PluginError};

/// What an uploader is given.
pub struct UploadInput {
    pub file: PathBuf,
    pub title: String,
    pub description: String,
    /// The access token or key the user entered for this plugin.
    pub token: String,
}

pub struct UploadOutput {
    /// The address of the uploaded video, when the script returns one.
    pub url: Option<String>,
    /// Messages from `tempo.notify`, in order.
    pub messages: Vec<String>,
}

/// Check a URL against the plugin's host list. Returns true when it is plain
/// HTTP, which is only accepted for this computer.
fn check_url(url: &str, allowed: &[String]) -> Result<bool, String> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("the address contains a space or control character".into());
    }
    let (scheme, rest) = url.split_once("://").ok_or("the address must start with https://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains(['@', '\\', '[']) {
        return Err("the address must be a plain host name".into());
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h).to_ascii_lowercase();
    if !allowed.iter().any(|a| a.eq_ignore_ascii_case(&host)) {
        return Err(format!("this plugin has no permission to contact {host}"));
    }
    match scheme {
        "https" => Ok(false),
        "http" if host == "localhost" || host == "127.0.0.1" => Ok(true),
        _ => Err("only https:// addresses are allowed".into()),
    }
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// Split curl's `-i` output into the final status, headers and body.
fn parse_response(mut raw: &[u8]) -> Option<Response> {
    loop {
        let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
        let head = String::from_utf8_lossy(&raw[..end]).into_owned();
        raw = &raw[end + 4..];
        let mut lines = head.lines();
        let status: u16 = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
        // "100 Continue" and friends come before the real answer.
        if (100..200).contains(&status) {
            continue;
        }
        let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
        return Some(Response { status, headers, body: raw.to_vec() });
    }
}

enum Payload<'a> {
    None,
    Body(Vec<u8>),
    /// The exported file as the request body.
    File(&'a Path),
    /// The exported file as one field of a form, with text fields beside it.
    Form(&'a Path, String, Vec<(String, String)>),
}

/// Make one request with curl. Blocking.
fn request(url: &str, plain_http: bool, method: &str, headers: &[(String, String)], payload: Payload) -> Result<Response, String> {
    let mut curl = Command::new("curl");
    // -q: ignore the user's curl settings. No -L: a redirect could leave the allowed hosts.
    curl.args(["-q", "-sS", "-i", "-g", "--max-time", "3600", "--proto", if plain_http { "=http,https" } else { "=https" }]);
    curl.args(["-X", method, "-H", "Expect:"]);
    for (name, value) in headers {
        curl.args(["-H", &format!("{name}: {value}")]);
    }
    curl.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut body = None;
    match payload {
        Payload::None => {}
        Payload::Body(bytes) => {
            curl.args(["--data-binary", "@-"]).stdin(Stdio::piped());
            body = Some(bytes);
        }
        Payload::File(path) => {
            // curl streams the file, so it is never held in memory. `-g` above
            // stops it reading brackets in the name as a pattern.
            curl.arg("-T").arg(path);
        }
        Payload::Form(path, field, fields) => {
            for (name, value) in fields {
                curl.args(["--form-string", &format!("{name}={value}")]);
            }
            let quoted = path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
            curl.args(["-F", &format!("{field}=@\"{quoted}\"")]);
        }
    }
    curl.arg("--").arg(url);
    let mut child = curl.spawn().map_err(|e| format!("cannot run curl: {e}"))?;
    if let (Some(bytes), Some(mut stdin)) = (body, child.stdin.take()) {
        // curl may stop reading early on an error; its own message is reported below.
        let _ = stdin.write_all(&bytes);
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    parse_response(&output.stdout).ok_or_else(|| {
        let reason = first_line(&String::from_utf8_lossy(&output.stderr));
        format!("the request failed: {}", reason.trim_start_matches("curl: "))
    })
}

fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> mlua::Result<Value> {
    Ok(match value {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::String(lua.create_string(s)?),
        serde_json::Value::Array(items) => {
            let t = lua.create_table()?;
            for item in items {
                t.push(json_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
        serde_json::Value::Object(map) => {
            let t = lua.create_table()?;
            for (key, item) in map {
                t.set(key.as_str(), json_to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
    })
}

fn lua_to_json(value: Value, depth: usize) -> mlua::Result<serde_json::Value> {
    if depth > 32 {
        return Err(script_error("the table is nested too deeply to encode"));
    }
    Ok(match value {
        Value::Nil => serde_json::Value::Null,
        Value::Boolean(b) => b.into(),
        Value::Integer(i) => i.into(),
        Value::Number(n) => serde_json::Number::from_f64(n).map_or(serde_json::Value::Null, Into::into),
        Value::String(s) => s.to_string_lossy().into(),
        // A table with a first element is a list; any other table is an object.
        Value::Table(t) if t.raw_len() > 0 => {
            serde_json::Value::Array(t.sequence_values::<Value>().map(|v| lua_to_json(v?, depth + 1)).collect::<mlua::Result<_>>()?)
        }
        Value::Table(t) => {
            let mut map = serde_json::Map::new();
            for pair in t.pairs::<String, Value>() {
                let (key, item) = pair?;
                map.insert(key, lua_to_json(item, depth + 1)?);
            }
            serde_json::Value::Object(map)
        }
        _ => return Err(script_error("only numbers, text, true/false and tables can be encoded")),
    })
}

fn string_pairs(table: Option<Table>) -> mlua::Result<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    let Some(table) = table else { return Ok(pairs) };
    for pair in table.pairs::<String, String>() {
        let (name, value) = pair?;
        // Plain names only: curl gives a leading `@` and other marks a special meaning.
        let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !plain || value.contains(['\r', '\n']) {
            return Err(script_error(format!("'{name}' is not a valid header or field")));
        }
        pairs.push((name, value));
    }
    Ok(pairs)
}

/// Run one uploader of a plugin. Blocking; call from a worker thread.
pub fn run_upload(plugin: &Plugin, function: &str, input: UploadInput) -> Result<UploadOutput, PluginError> {
    let fail = |e: mlua::Error| PluginError::Script(first_line(&e.to_string()));
    let (lua, deadline) = sandbox().map_err(fail)?;
    let messages = Rc::new(RefCell::new(Vec::new()));
    let size = std::fs::metadata(&input.file).map_err(|e| PluginError::Io(input.file.clone(), e))?.len();

    let build = || -> mlua::Result<Option<String>> {
        let tempo = lua.create_table()?;
        let m = messages.clone();
        tempo.set("notify", lua.create_function(move |_, text: String| {
            m.borrow_mut().push(text);
            Ok(())
        })?)?;

        // Both http functions share this: check the address, run curl, and give
        // the waiting time back to the script's time limit.
        let send = {
            let allowed = plugin.network.clone();
            let file = input.file.clone();
            let deadline: Rc<Cell<Instant>> = deadline.clone();
            Rc::new(move |lua: &Lua, options: Table, with_file: bool| -> mlua::Result<Table> {
                let url: String = options.get("url")?;
                let plain_http = check_url(&url, &allowed).map_err(script_error)?;
                let headers = string_pairs(options.get("headers")?)?;
                let body: Option<mlua::String> = options.get("body")?;
                let payload = if with_file {
                    match options.get::<Option<String>>("field")? {
                        Some(field) if !field.is_empty() && field.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') => Payload::Form(&file, field, string_pairs(options.get("form")?)?),
                        Some(_) => return Err(script_error("the form field name is not valid")),
                        None => Payload::File(&file),
                    }
                } else {
                    body.map_or(Payload::None, |b| Payload::Body(b.as_bytes().to_vec()))
                };
                let default = if with_file { "POST" } else if matches!(payload, Payload::None) { "GET" } else { "POST" };
                let method = options.get::<Option<String>>("method")?.unwrap_or_else(|| default.to_string()).to_ascii_uppercase();
                if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str()) {
                    return Err(script_error(format!("'{method}' is not an allowed method")));
                }
                let started = Instant::now();
                let response = request(&url, plain_http, &method, &headers, payload);
                deadline.set(deadline.get() + started.elapsed());
                let response = response.map_err(script_error)?;
                let t = lua.create_table()?;
                t.set("status", response.status)?;
                t.set("body", lua.create_string(&response.body)?)?;
                let h = lua.create_table()?;
                for (name, value) in response.headers {
                    h.set(name, value)?;
                }
                t.set("headers", h)?;
                Ok(t)
            })
        };
        let http = lua.create_table()?;
        let s = send.clone();
        http.set("request", lua.create_function(move |lua, options: Table| s(lua, options, false))?)?;
        http.set("upload", lua.create_function(move |lua, options: Table| send(lua, options, true))?)?;
        tempo.set("http", http)?;

        let json = lua.create_table()?;
        json.set("decode", lua.create_function(|lua, text: mlua::String| match serde_json::from_slice::<serde_json::Value>(&text.as_bytes()) {
            Ok(value) => json_to_lua(lua, &value),
            // Not JSON: nil, so a script can test the result.
            Err(_) => Ok(Value::Nil),
        })?)?;
        json.set("encode", lua.create_function(|_, value: Value| Ok(lua_to_json(value, 0)?.to_string()))?)?;
        tempo.set("json", json)?;
        lua.globals().set("tempo", tempo)?;

        lua.load(&*plugin.script).set_name(plugin.name.as_str()).exec()?;
        let entry: mlua::Function = lua.globals().get(function).map_err(|_| script_error(format!("the plugin has no function called '{function}'")))?;
        let video = lua.create_table()?;
        video.set("title", input.title.as_str())?;
        video.set("description", input.description.as_str())?;
        video.set("token", input.token.as_str())?;
        video.set("file_name", input.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())?;
        video.set("size", size)?;
        entry.call::<Option<String>>(video)
    };
    let url = build().map_err(fail)?;
    drop(lua);
    let messages = messages.take();
    Ok(UploadOutput { url, messages })
}
