//! Project Manager: a grid/list of projects and folders with actions at the bottom,
//! as in DaVinci Resolve's Project Manager.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gdk, gio};
use libadwaita as adw;
use libadwaita::prelude::*;
use serde::{Deserialize, Serialize};
use tempo_timeline::Project;

use crate::util::{label, pill, tool_button, tool_toggle};

const DEFAULT_THUMB_WIDTH: i32 = 224;
const MIN_THUMB_WIDTH: f64 = 160.0;
const MAX_THUMB_WIDTH: f64 = 320.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectThumb {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub folder: Option<String>,
    /// Filled in when the list is loaded; not stored.
    #[serde(skip)]
    pub modified: Option<std::time::SystemTime>,
    #[serde(skip)]
    pub missing: bool,
    /// Loaded from `<user_cache_dir>/tempo/project-thumbs/<id>.rgba`; not stored in JSON.
    #[serde(skip)]
    pub thumbnail: Option<ProjectThumb>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectIndex {
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StoredIndex {
    Index(ProjectIndex),
    Legacy(Vec<Entry>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    DateModified,
    NameAsc,
    NameDesc,
}

impl SortMode {
    fn from_index(index: u32) -> Self {
        match index {
            1 => Self::NameAsc,
            2 => Self::NameDesc,
            _ => Self::DateModified,
        }
    }
}

#[derive(Debug, Clone)]
enum ManagerItem {
    Folder { name: String, count: usize },
    Project(Entry),
}

fn list_path() -> PathBuf {
    glib::user_data_dir().join("tempo").join("projects.json")
}

/// Where a project's rolling backups live: user data, never beside the user's media.
pub fn backup_dir(project_id: uuid::Uuid) -> PathBuf {
    glib::user_data_dir().join("tempo").join("backups").join(project_id.to_string())
}

fn thumb_dir() -> PathBuf {
    glib::user_cache_dir().join("tempo").join("project-thumbs")
}

fn thumb_path(dir: &Path, project_id: uuid::Uuid) -> PathBuf {
    dir.join(format!("{project_id}.rgba"))
}

fn thumb_dims(data: &[u8]) -> Option<(u32, u32)> {
    [(160, 90), (320, 180), (96, 54)]
        .into_iter()
        .find(|&(w, h)| (w as usize) * (h as usize) * 4 == data.len())
}

fn encode_thumb(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + rgba.len());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(rgba);
    out
}

fn decode_thumb(bytes: &[u8]) -> Option<ProjectThumb> {
    if bytes.len() < 8 {
        return None;
    }
    let width = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let height = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let expected = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
    if width == 0 || height == 0 || bytes.len() - 8 != expected {
        return None;
    }
    Some(ProjectThumb {
        width,
        height,
        rgba: bytes[8..].to_vec(),
    })
}

fn save_project_thumbnail_in(project: &Project, dir: &Path) {
    let mut sources: Vec<_> = project.sources.values().collect();
    sources.sort_by_key(|s| (s.import_order, s.id));
    let Some((w, h, rgba)) = sources.into_iter().find_map(|s| {
        let data = s.thumbnail_data.as_deref()?;
        let (w, h) = thumb_dims(data)?;
        Some((w, h, data))
    }) else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!("could not create thumbnail cache dir: {e}");
        return;
    }
    let path = thumb_path(dir, project.id);
    if let Err(e) = std::fs::write(&path, encode_thumb(w, h, rgba)) {
        tracing::warn!("could not save project thumbnail {}: {e}", path.display());
    }
}

fn load_thumbnail_in(dir: &Path, project_id: uuid::Uuid) -> Option<ProjectThumb> {
    let bytes = std::fs::read(thumb_path(dir, project_id)).ok()?;
    decode_thumb(&bytes)
}

/// Persist the first source's thumbnail RGBA to the project thumbnail cache. Blocking: call from a worker.
pub fn save_project_thumbnail(project: &Project) {
    save_project_thumbnail_in(project, &thumb_dir());
}

/// Case-insensitive duplicate name check (matching DaVinci Resolve).
pub fn is_duplicate_name(existing: &[String], candidate: &str) -> bool {
    let candidate = candidate.trim().to_lowercase();
    !candidate.is_empty() && existing.iter().any(|n| n.trim().to_lowercase() == candidate)
}

/// First available `"Untitled Project"`, `"Untitled Project 2"`, ... (case-insensitive).
pub fn next_default_project_name(existing: &[String]) -> String {
    next_available_name(existing, "Untitled Project")
}

fn next_available_name(existing: &[String], base: &str) -> String {
    if !is_duplicate_name(existing, base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !is_duplicate_name(existing, candidate))
        .unwrap_or_else(|| format!("{base} 2"))
}

fn parse_index(text: &str) -> ProjectIndex {
    let mut index = match serde_json::from_str::<StoredIndex>(text) {
        Ok(StoredIndex::Index(idx)) => idx,
        Ok(StoredIndex::Legacy(entries)) => ProjectIndex { folders: Vec::new(), entries },
        Err(_) => ProjectIndex::default(),
    };
    normalize_folders(&mut index);
    index
}

fn normalize_folders(index: &mut ProjectIndex) {
    let mut seen: Vec<String> = Vec::new();
    for f in index.folders.drain(..) {
        let trimmed = f.trim().to_string();
        if !trimmed.is_empty() && !is_duplicate_name(&seen, &trimmed) {
            seen.push(trimmed);
        }
    }
    for e in &mut index.entries {
        if let Some(ref f) = e.folder {
            let trimmed = f.trim();
            if trimmed.is_empty() {
                e.folder = None;
            } else if let Some(existing) = seen.iter().find(|s| s.eq_ignore_ascii_case(trimmed)) {
                e.folder = Some(existing.clone());
            } else {
                seen.push(trimmed.to_string());
                e.folder = Some(trimmed.to_string());
            }
        }
    }
    index.folders = seen;
}

fn load_raw_index_at(path: &Path) -> ProjectIndex {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| parse_index(&text))
        .unwrap_or_default()
}

fn save_index_at(path: &Path, index: &ProjectIndex) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(index).unwrap_or_else(|_| "{}".into());
    std::fs::write(path, text)
}

fn load_index_at(path: &Path, thumbs: &Path) -> ProjectIndex {
    let mut index = load_raw_index_at(path);
    for e in &mut index.entries {
        match std::fs::metadata(&e.path).and_then(|m| m.modified()) {
            Ok(time) => {
                e.modified = Some(time);
                e.thumbnail = tempo_project::read_project_id(&e.path)
                    .ok()
                    .and_then(|id| load_thumbnail_in(thumbs, id));
            }
            Err(_) => e.missing = true,
        }
    }
    sort_entries(&mut index.entries, SortMode::DateModified);
    index
}

/// Read the project index (folders + entries). Blocking: call from a worker.
pub fn load_index() -> ProjectIndex {
    load_index_at(&list_path(), &thumb_dir())
}

/// Read the project list. Blocking: call from a worker.
pub fn load_list() -> Vec<Entry> {
    load_index().entries
}

fn sort_entries(entries: &mut [Entry], mode: SortMode) {
    match mode {
        SortMode::DateModified => {
            entries.sort_by(|a, b| {
                b.modified
                    .cmp(&a.modified)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
        }
        SortMode::NameAsc => {
            entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
        }
        SortMode::NameDesc => {
            entries.sort_by(|a, b| b.name.to_lowercase().cmp(&a.name.to_lowercase()).then_with(|| b.name.cmp(&a.name)));
        }
    }
}

fn sort_folders(folders: &mut [String], mode: SortMode) {
    match mode {
        SortMode::NameDesc => {
            folders.sort_by(|a, b| b.to_lowercase().cmp(&a.to_lowercase()).then_with(|| b.cmp(a)));
        }
        SortMode::DateModified | SortMode::NameAsc => {
            folders.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
        }
    }
}

fn remember_at(list_file: &Path, name: &str, path: &Path, default_folder: Option<&str>) {
    let mut index = load_raw_index_at(list_file);
    let existing_folder = index
        .entries
        .iter()
        .find(|e| e.path == path)
        .and_then(|e| e.folder.clone())
        .or_else(|| default_folder.map(str::to_string));
    index.entries.retain(|e| e.path != path);
    index.entries.insert(
        0,
        Entry {
            name: name.to_string(),
            path: path.to_path_buf(),
            folder: existing_folder,
            modified: None,
            missing: false,
            thumbnail: None,
        },
    );
    normalize_folders(&mut index);
    if let Err(e) = save_index_at(list_file, &index) {
        tracing::warn!("could not save the project list: {e}");
    }
}

/// Add or refresh a project in the list. Blocking: call from a worker.
pub fn remember(name: &str, path: &Path) {
    remember_at(&list_path(), name, path, None);
}

/// Like [`remember`], but a project new to the list goes into `folder`. Blocking.
pub fn remember_in(name: &str, path: &Path, folder: Option<&str>) {
    remember_at(&list_path(), name, path, folder);
}

/// Whether a project other than the one at `except` already has `name` (case-insensitive).
fn name_in_use(index: &ProjectIndex, except: &Path, name: &str) -> bool {
    let name = name.trim().to_lowercase();
    index.entries.iter().any(|e| e.path != except && e.name.trim().to_lowercase() == name)
}

/// Name and id for Save Project As to `new_path`. The name follows the file name typed,
/// but keeping the suggested one is not a rename. A different file gets a new id so its
/// backups and thumbnail don't mix with (and prune) the original's. Blocking.
pub fn save_as_identity(
    name: &str,
    id: uuid::Uuid,
    current_path: Option<&Path>,
    new_path: &Path,
) -> Result<(String, uuid::Uuid), String> {
    save_as_identity_at(&list_path(), name, id, current_path, new_path)
}

fn save_as_identity_at(
    list_file: &Path,
    name: &str,
    id: uuid::Uuid,
    current_path: Option<&Path>,
    new_path: &Path,
) -> Result<(String, uuid::Uuid), String> {
    let stem = new_path.file_stem().map(|s| s.to_string_lossy().trim().to_string()).unwrap_or_default();
    let new_name = if stem.is_empty() || stem == safe_filename(name).trim() { name.to_string() } else { stem };
    let same_file = current_path == Some(new_path);
    if !(same_file && new_name == name) && name_in_use(&load_raw_index_at(list_file), new_path, &new_name) {
        return Err("A project with the same name is already in use.".into());
    }
    Ok((new_name, if same_file { id } else { uuid::Uuid::new_v4() }))
}

/// Add `.tempo` unless the file name already ends in it. Appends rather than replaces,
/// so "My.Film" becomes "My.Film.tempo", not "My.tempo".
pub fn with_tempo_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|e| e == "tempo") {
        return path;
    }
    let mut s = path.into_os_string();
    s.push(".tempo");
    s.into()
}

/// Copy a project file for Export. Refuses the project's own file (or a link to it):
/// `fs::copy` truncates the target before reading, which would empty the project. Blocking.
fn export_project_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if let (Ok(a), Ok(b)) = (std::fs::metadata(src), std::fs::metadata(dest)) {
        if a.dev() == b.dev() && a.ino() == b.ino() {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "that is the project's own file"));
        }
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(src, dest).map(|_| ())
}

fn forget_at(list_file: &Path, path: &Path) {
    let mut index = load_raw_index_at(list_file);
    index.entries.retain(|e| e.path != path);
    if let Err(e) = save_index_at(list_file, &index) {
        tracing::warn!("could not save the project list: {e}");
    }
}

fn forget(path: &Path) {
    forget_at(&list_path(), path);
}

fn create_folder_in(index: &mut ProjectIndex, name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Folder name cannot be empty.".into());
    }
    if is_duplicate_name(&index.folders, trimmed) {
        return Err("A folder with that name already exists.".into());
    }
    index.folders.push(trimmed.to_string());
    Ok(trimmed.to_string())
}

fn rename_folder_in(index: &mut ProjectIndex, old_name: &str, new_name: &str) -> Result<String, String> {
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err("Folder name cannot be empty.".into());
    }
    if !old_name.eq_ignore_ascii_case(trimmed) && is_duplicate_name(&index.folders, trimmed) {
        return Err("A folder with that name already exists.".into());
    }
    let Some(slot) = index.folders.iter_mut().find(|f| *f == old_name) else {
        return Err(format!("Folder \"{old_name}\" was not found."));
    };
    *slot = trimmed.to_string();
    for e in &mut index.entries {
        if e.folder.as_deref() == Some(old_name) {
            e.folder = Some(trimmed.to_string());
        }
    }
    Ok(trimmed.to_string())
}

fn delete_folder_in(index: &mut ProjectIndex, folder_name: &str) {
    index.folders.retain(|f| f != folder_name);
    for e in &mut index.entries {
        if e.folder.as_deref() == Some(folder_name) {
            e.folder = None;
        }
    }
}

fn move_entry_to_folder_in(index: &mut ProjectIndex, path: &Path, folder: Option<&str>) {
    let target = folder.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(ref f) = target {
        if !index.folders.iter().any(|existing| existing == f) {
            index.folders.push(f.clone());
        }
    }
    for e in &mut index.entries {
        if e.path == path {
            e.folder = target.clone();
        }
    }
}

fn update_index_at(list_file: &Path, f: impl FnOnce(&mut ProjectIndex) -> Result<(), String>) -> Result<(), String> {
    let mut index = load_raw_index_at(list_file);
    f(&mut index)?;
    save_index_at(list_file, &index).map_err(|e| e.to_string())
}

/// Change the name stored in a project file. The file itself keeps its name. Blocking.
fn rename_project(path: &Path, name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Project name cannot be empty.".into());
    }
    if name_in_use(&load_raw_index_at(&list_path()), path, trimmed) {
        return Err("A project with the same name is already in use.".into());
    }
    let mut project = tempo_project::load_project(path).map_err(|e| e.to_string())?;
    project.name = trimmed.to_string();
    tempo_project::save_project(&project, path).map_err(|e| e.to_string())?;
    remember(trimmed, path);
    Ok(())
}

/// Copy a project file next to the original as "<name> copy". Blocking.
fn duplicate_project(path: &Path) -> Result<(), String> {
    let list_file = list_path();
    let index = load_raw_index_at(&list_file);
    let folder = index.entries.iter().find(|e| e.path == path).and_then(|e| e.folder.clone());
    let existing_names: Vec<String> = index.entries.iter().map(|e| e.name.clone()).collect();

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Project".into());
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut copy = dir.join(format!("{stem} copy.tempo"));
    let mut n = 2;
    while copy.exists() {
        copy = dir.join(format!("{stem} copy {n}.tempo"));
        n += 1;
    }
    std::fs::copy(path, &copy).map_err(|e| e.to_string())?;
    let mut project = tempo_project::load_project(&copy).map_err(|e| e.to_string())?;
    project.name = next_available_name(&existing_names, &format!("{} copy", project.name));
    // A new project of its own, with its own backups.
    project.id = uuid::Uuid::new_v4();
    tempo_project::save_project(&project, &copy).map_err(|e| e.to_string())?;
    remember_at(&list_file, &project.name, &copy, folder.as_deref());
    Ok(())
}

/// Copy a backup next to `original` as "<name> (backup <time>)" and return the copy's
/// path and project name. The original file is never touched, even if it is damaged. Blocking.
fn restore_backup(backup: &Path, taken: &str, original: &Path) -> Result<(PathBuf, String), String> {
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Project".into());
    let dir = original.parent().unwrap_or(Path::new("."));
    let copy = dir.join(format!("{stem} (backup {taken}).tempo"));
    if copy.exists() {
        return Err(format!("{} already exists", copy.display()));
    }
    std::fs::copy(backup, &copy).map_err(|e| e.to_string())?;
    let mut project = tempo_project::load_project(&copy).map_err(|e| e.to_string())?;
    project.name = format!("{} (backup {taken})", project.name);
    // Its own id, so its backups don't mix with (and prune) the original's.
    project.id = uuid::Uuid::new_v4();
    tempo_project::save_project(&project, &copy).map_err(|e| e.to_string())?;
    Ok((copy, project.name))
}

fn relative_time(time: Option<std::time::SystemTime>) -> String {
    let Some(secs) = time.and_then(|t| t.elapsed().ok()).map(|d| d.as_secs()) else {
        return String::new();
    };
    match secs {
        0..=59 => "Just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        86_400..=172_799 => "Yesterday".into(),
        _ => format!("{} days ago", secs / 86_400),
    }
}

fn thumb_texture(thumb: &ProjectThumb) -> gdk::Texture {
    let bytes = glib::Bytes::from(&thumb.rgba);
    gdk::MemoryTexture::new(
        thumb.width as i32,
        thumb.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        thumb.width as usize * 4,
    )
    .upcast()
}

fn safe_filename(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
    if cleaned.trim().is_empty() {
        "Project".into()
    } else {
        cleaned
    }
}

type CellMenuFn = Rc<RefCell<Option<Box<dyn Fn(u32, &gtk::Widget, f64, f64)>>>>;

pub struct ProjectManager {
    pub root: gtk::Box,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    stack: gtk::Stack,
    back_btn: gtk::Button,
    heading: gtk::Label,
    search: gtk::SearchEntry,
    sort_dropdown: gtk::DropDown,
    grid_btn: gtk::ToggleButton,
    open: gtk::Button,
    export: gtk::Button,
    index: RefCell<ProjectIndex>,
    current_folder: RefCell<Option<String>>,
    thumb_width: Rc<Cell<i32>>,
    card_thumbs: Rc<RefCell<Vec<(glib::WeakRef<gtk::Overlay>, glib::WeakRef<gtk::Label>)>>>,
}

impl ProjectManager {
    /// `on_open` receives a project path; `on_new` asks for the New Project dialog,
    /// with the folder that is open (the new project goes there).
    pub fn new(on_open: impl Fn(PathBuf) + Clone + 'static, on_new: impl Fn(Option<String>) + 'static) -> Rc<Self> {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["tempo-panel"])
            .build();

        let title_bar = gtk::CenterBox::builder()
            .css_classes(["tempo-bar", "tempo-sep-bottom"])
            .height_request(44)
            .build();
        title_bar.set_center_widget(Some(&label("Tempo", &["tempo-heading", "tempo-bright"])));
        title_bar.set_end_widget(Some(&gtk::WindowControls::new(gtk::PackType::End)));
        let handle = gtk::WindowHandle::builder().child(&title_bar).build();
        root.append(&handle);

        let back_btn = tool_button("go-previous-symbolic", "Back to Projects");
        back_btn.set_visible(false);
        let heading = label("Projects", &["tempo-heading", "tempo-bright"]);
        heading.set_hexpand(true);
        heading.set_xalign(0.0);

        let new_folder_btn = tool_button("folder-new-symbolic", "New Folder");

        let size_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, MIN_THUMB_WIDTH, MAX_THUMB_WIDTH, 8.0);
        size_scale.set_value(DEFAULT_THUMB_WIDTH as f64);
        size_scale.set_width_request(110);
        size_scale.set_valign(gtk::Align::Center);
        size_scale.set_tooltip_text(Some("Thumbnail size"));
        size_scale.update_property(&[gtk::accessible::Property::Label("Thumbnail size")]);

        let sort_dropdown = gtk::DropDown::from_strings(&["Date Modified", "Name A–Z", "Name Z–A"]);
        sort_dropdown.set_valign(gtk::Align::Center);
        sort_dropdown.set_tooltip_text(Some("Sort projects"));
        sort_dropdown.update_property(&[gtk::accessible::Property::Label("Sort projects")]);

        let grid_btn = tool_toggle("view-grid-symbolic", "Grid View");
        let list_btn = tool_toggle("view-list-symbolic", "List View");
        list_btn.set_group(Some(&grid_btn));
        grid_btn.set_active(true);
        let view_toggle_box = gtk::Box::builder().css_classes(["linked"]).valign(gtk::Align::Center).build();
        view_toggle_box.append(&grid_btn);
        view_toggle_box.append(&list_btn);

        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search projects")
            .width_request(240)
            .build();
        let search_toggle = tool_toggle("system-search-symbolic", "Search projects");
        search.set_visible(false);

        let header = gtk::Box::builder()
            .spacing(8)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(16)
            .margin_end(16)
            .build();
        header.append(&back_btn);
        header.append(&heading);
        header.append(&new_folder_btn);
        header.append(&size_scale);
        header.append(&sort_dropdown);
        header.append(&view_toggle_box);
        header.append(&search);
        header.append(&search_toggle);
        root.append(&header);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::builder()
            .model(&store)
            .autoselect(false)
            .can_unselect(true)
            .build();

        let thumb_width = Rc::new(Cell::new(DEFAULT_THUMB_WIDTH));
        let card_thumbs = Rc::new(RefCell::new(Vec::new()));
        let grid_menu_cb: CellMenuFn = Rc::new(RefCell::new(None));
        let list_menu_cb: CellMenuFn = Rc::new(RefCell::new(None));

        let grid = gtk::GridView::builder()
            .model(&selection)
            .factory(&card_factory(&selection, thumb_width.clone(), card_thumbs.clone(), grid_menu_cb.clone()))
            .max_columns(8)
            .min_columns(1)
            .css_classes(["projects"])
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let scrolled_grid = gtk::ScrolledWindow::builder()
            .child(&grid)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let column_view = build_column_view(&selection, list_menu_cb.clone());
        let scrolled_list = gtk::ScrolledWindow::builder()
            .child(&column_view)
            .vexpand(true)
            .build();

        let empty = label("No projects yet", &["empty-hint"]);
        empty.set_vexpand(true);
        let stack = gtk::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scrolled_grid, Some("grid"));
        stack.add_named(&scrolled_list, Some("list"));
        root.append(&stack);

        let export = pill("Export");
        export.set_tooltip_text(Some("Save a copy of the selected .tempo project file"));
        export.set_sensitive(false);
        let import = pill("Import");
        import.set_tooltip_text(Some("Open a .tempo file from anywhere"));
        let left = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        left.append(&export);
        left.append(&import);

        let new = pill("New Project");
        let open = pill("Open");
        open.add_css_class("default");
        open.set_sensitive(false);
        let right = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        right.append(&new);
        right.append(&open);
        let footer = gtk::CenterBox::builder()
            .margin_top(12)
            .margin_bottom(16)
            .margin_start(24)
            .margin_end(24)
            .build();
        footer.set_start_widget(Some(&left));
        footer.set_end_widget(Some(&right));
        root.append(&footer);

        let pm = Rc::new(Self {
            root,
            store,
            selection,
            stack,
            back_btn,
            heading,
            search,
            sort_dropdown,
            grid_btn,
            open,
            export,
            index: Default::default(),
            current_folder: RefCell::new(None),
            thumb_width,
            card_thumbs,
        });

        let p = pm.clone();
        search_toggle.connect_toggled(move |b| {
            p.search.set_visible(b.is_active());
            if b.is_active() {
                p.search.grab_focus();
            } else {
                p.search.set_text("");
            }
        });
        let p = pm.clone();
        pm.search.connect_search_changed(move |_| p.show_filtered());

        let p = pm.clone();
        pm.back_btn.connect_clicked(move |_| {
            *p.current_folder.borrow_mut() = None;
            p.show_filtered();
        });

        let p = pm.clone();
        new_folder_btn.connect_clicked(move |b| {
            p.ask_new_folder(b.upcast_ref());
        });

        let p = pm.clone();
        size_scale.connect_value_changed(move |scale| {
            let w = scale.value().round().clamp(MIN_THUMB_WIDTH, MAX_THUMB_WIDTH) as i32;
            let h = (w * 9 / 16).max(1);
            let chars = (w / 10).max(14);
            p.thumb_width.set(w);
            p.card_thumbs.borrow_mut().retain(|(thumb_w, label_w)| {
                let Some(thumb) = thumb_w.upgrade() else { return false };
                thumb.set_size_request(w, h);
                if let Some(lbl) = label_w.upgrade() {
                    lbl.set_max_width_chars(chars);
                }
                true
            });
        });

        let p = pm.clone();
        pm.sort_dropdown.connect_selected_notify(move |_| p.show_filtered());

        let p = pm.clone();
        pm.grid_btn.connect_toggled(move |_| p.update_stack_view());
        let p = pm.clone();
        list_btn.connect_toggled(move |_| p.update_stack_view());

        let p = pm.clone();
        pm.selection.connect_selected_item_notify(move |sel| {
            let item = sel
                .selected_item()
                .and_downcast::<glib::BoxedAnyObject>()
                .map(|o| o.borrow::<ManagerItem>().clone());
            p.open.set_sensitive(item.is_some());
            p.export.set_sensitive(matches!(item, Some(ManagerItem::Project(e)) if !e.missing));
        });

        let p = pm.clone();
        new.connect_clicked(move |_| on_new(p.current_folder.borrow().clone()));

        let p = pm.clone();
        let open_cb = on_open.clone();
        pm.open.connect_clicked(move |_| {
            if let Some(item) = p.selected_item() {
                p.activate_item(item, &open_cb);
            }
        });

        let p = pm.clone();
        let open_cb = on_open.clone();
        grid.connect_activate(move |_, pos| {
            if let Some(obj) = p.store.item(pos).and_downcast::<glib::BoxedAnyObject>() {
                let item = obj.borrow::<ManagerItem>().clone();
                p.activate_item(item, &open_cb);
            }
        });

        let p = pm.clone();
        let open_cb = on_open.clone();
        column_view.connect_activate(move |_, pos| {
            if let Some(obj) = p.store.item(pos).and_downcast::<glib::BoxedAnyObject>() {
                let item = obj.borrow::<ManagerItem>().clone();
                p.activate_item(item, &open_cb);
            }
        });

        let open_cb = on_open.clone();
        import.connect_clicked(move |b| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Tempo projects"));
            filter.add_pattern("*.tempo");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder()
                .title("Import Project")
                .modal(true)
                .filters(&filters)
                .build();
            let open_cb = open_cb.clone();
            dialog.open(b.root().and_downcast::<gtk::Window>().as_ref(), gio::Cancellable::NONE, move |result| {
                if let Some(path) = result.ok().and_then(|f| f.path()) {
                    open_cb(path);
                }
            });
        });

        let p = pm.clone();
        pm.export.connect_clicked(move |b| {
            let Some(ManagerItem::Project(entry)) = p.selected_item() else { return };
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Tempo projects"));
            filter.add_pattern("*.tempo");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let initial_name = format!("{}.tempo", safe_filename(&entry.name));
            let dialog = gtk::FileDialog::builder()
                .title("Export Project")
                .modal(true)
                .filters(&filters)
                .default_filter(&filter)
                .initial_name(&initial_name)
                .build();
            let pm = p.clone();
            let src = entry.path.clone();
            dialog.save(b.root().and_downcast::<gtk::Window>().as_ref(), gio::Cancellable::NONE, move |result| {
                let Some(dest) = result.ok().and_then(|f| f.path()).map(with_tempo_extension) else { return };
                let pm = pm.clone();
                let src = src.clone();
                glib::MainContext::default().spawn_local(async move {
                    let res = gio::spawn_blocking(move || export_project_file(&src, &dest)).await;
                    match res {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => pm.report(&format!("Could not export the project: {e}")),
                        Err(_) => pm.report("Exporting the project stopped unexpectedly."),
                    }
                });
            });
        });

        // Right-click menu handler shared by both GridView and ColumnView cells.
        let pm_grid = pm.clone();
        let grid_w: gtk::Widget = grid.clone().upcast();
        let open_cb_grid = on_open.clone();
        *grid_menu_cb.borrow_mut() = Some(Box::new(move |pos, cell, x, y| {
            pm_grid.selection.set_selected(pos);
            let (vx, vy) = cell
                .compute_point(&grid_w, &gtk::graphene::Point::new(x as f32, y as f32))
                .map(|pt| (pt.x() as f64, pt.y() as f64))
                .unwrap_or((x, y));
            pm_grid.show_context_menu(&grid_w, vx, vy, &open_cb_grid);
        }));

        let pm_list = pm.clone();
        let list_w: gtk::Widget = column_view.clone().upcast();
        let open_cb_list = on_open.clone();
        *list_menu_cb.borrow_mut() = Some(Box::new(move |pos, cell, x, y| {
            pm_list.selection.set_selected(pos);
            let (vx, vy) = cell
                .compute_point(&list_w, &gtk::graphene::Point::new(x as f32, y as f32))
                .map(|pt| (pt.x() as f64, pt.y() as f64))
                .unwrap_or((x, y));
            pm_list.show_context_menu(&list_w, vx, vy, &open_cb_list);
        }));

        pm.reload();
        pm
    }

    fn activate_item(&self, item: ManagerItem, on_open: &impl Fn(PathBuf)) {
        match item {
            ManagerItem::Folder { name, .. } => {
                *self.current_folder.borrow_mut() = Some(name);
                self.show_filtered();
            }
            ManagerItem::Project(entry) => {
                on_open(entry.path);
            }
        }
    }

    fn show_context_menu(self: &Rc<Self>, parent: &gtk::Widget, x: f64, y: f64, on_open: &(impl Fn(PathBuf) + Clone + 'static)) {
        let Some(selected) = self.selected_item() else { return };
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();
        let popover = gtk::Popover::builder().child(&list).has_arrow(false).build();
        popover.set_parent(parent);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

        let item = |text: &str, run: Box<dyn Fn()>| {
            let button = gtk::Button::builder()
                .label(text)
                .css_classes(["link-text"])
                .halign(gtk::Align::Start)
                .build();
            let pop = popover.clone();
            button.connect_clicked(move |_| {
                pop.popdown();
                run();
            });
            list.append(&button);
        };

        match selected {
            ManagerItem::Folder { name, .. } => {
                let (pm, fname) = (self.clone(), name.clone());
                item(
                    "Open",
                    Box::new(move || {
                        *pm.current_folder.borrow_mut() = Some(fname.clone());
                        pm.show_filtered();
                    }),
                );

                let (pm, fname, w) = (self.clone(), name.clone(), parent.clone());
                item("Rename…", Box::new(move || pm.ask_rename_folder(&w, &fname)));

                let (pm, fname) = (self.clone(), name);
                item(
                    "Delete Folder",
                    Box::new(move || {
                        let fname = fname.clone();
                        pm.run_index_then_reload(move |idx| {
                            delete_folder_in(idx, &fname);
                            Ok(())
                        });
                    }),
                );
            }
            ManagerItem::Project(entry) => {
                let (cb, path) = (on_open.clone(), entry.path.clone());
                item("Open", Box::new(move || cb(path.clone())));

                let (pm, e, p_widget) = (self.clone(), entry.clone(), parent.clone());
                item("Rename…", Box::new(move || pm.ask_rename(&p_widget, &e)));

                let (pm, path) = (self.clone(), entry.path.clone());
                item("Duplicate", Box::new(move || pm.run_then_reload(path.clone(), duplicate_project)));

                let (pm, e, p_widget) = (self.clone(), entry.clone(), parent.clone());
                item("Move to Folder…", Box::new(move || pm.ask_move_to_folder(&p_widget, &e)));

                let (path, p_widget) = (entry.path.clone(), parent.clone());
                item(
                    "Show in Files",
                    Box::new(move || {
                        let window = p_widget.root().and_downcast::<gtk::Window>();
                        gtk::FileLauncher::new(Some(&gio::File::for_path(&path))).open_containing_folder(window.as_ref(), gio::Cancellable::NONE, |_| {});
                    }),
                );

                let (pm, e, cb) = (self.clone(), entry.clone(), on_open.clone());
                item("Project Backups…", Box::new(move || pm.show_backups(&e, cb.clone())));

                // The file goes to the system trash, so this can be undone from the file manager.
                let (pm, path) = (self.clone(), entry.path.clone());
                item(
                    "Move to Trash",
                    Box::new(move || {
                        let (pm, path) = (pm.clone(), path.clone());
                        glib::MainContext::default().spawn_local(async move {
                            let check_path = path.clone();
                            let gone = gio::spawn_blocking(move || !check_path.exists()).await.unwrap_or(false);
                            match gio::File::for_path(&path).trash_future(glib::Priority::DEFAULT).await {
                                Err(e) if !gone => {
                                    pm.report(&format!("Could not move the project to the trash: {e}"));
                                    return;
                                }
                                _ => {}
                            }
                            let _ = gio::spawn_blocking(move || forget(&path)).await;
                            pm.reload();
                        });
                    }),
                );

                // Only the entry goes; the project file stays where it is.
                let (pm, path) = (self.clone(), entry.path.clone());
                item(
                    "Remove from List",
                    Box::new(move || {
                        pm.run_then_reload(path.clone(), |p| {
                            forget(p);
                            Ok(())
                        })
                    }),
                );
            }
        }

        popover.connect_closed(|p| p.unparent());
        popover.popup();
    }

    /// Run blocking work on a project file, then refresh the view.
    fn run_then_reload(self: &Rc<Self>, path: PathBuf, work: impl FnOnce(&Path) -> Result<(), String> + Send + 'static) {
        let pm = self.clone();
        glib::MainContext::default().spawn_local(async move {
            if let Ok(Err(e)) = gio::spawn_blocking(move || work(&path)).await {
                pm.report(&format!("That did not work: {e}"));
            }
            pm.reload();
        });
    }

    /// Run blocking work on `projects.json`, then refresh the view.
    fn run_index_then_reload(self: &Rc<Self>, work: impl FnOnce(&mut ProjectIndex) -> Result<(), String> + Send + 'static) {
        let pm = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let res = gio::spawn_blocking(move || update_index_at(&list_path(), work)).await;
            match res {
                Ok(Err(e)) => pm.report(&e),
                Err(_) => pm.report("Updating the project list stopped unexpectedly."),
                Ok(Ok(())) => {}
            }
            pm.reload();
        });
    }

    /// Tell the user about something that failed.
    fn report(&self, text: &str) {
        tracing::warn!("{text}");
        let dialog = adw::AlertDialog::builder().heading("Project Manager").body(text).build();
        dialog.add_responses(&[("ok", "OK")]);
        dialog.present(Some(&self.root));
    }

    fn ask_new_folder(self: &Rc<Self>, parent: &gtk::Widget) {
        let default_name = next_available_name(&self.index.borrow().folders, "Folder");
        let field = gtk::Entry::builder().text(&default_name).activates_default(true).build();
        let dialog = adw::AlertDialog::builder().heading("New Folder").extra_child(&field).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("create", "Create")]);
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("create"));
        dialog.set_close_response("cancel");
        let pm = self.clone();
        let f = field.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "create" {
                return;
            }
            let name = f.text().trim().to_string();
            if name.is_empty() {
                return;
            }
            *pm.current_folder.borrow_mut() = None;
            pm.run_index_then_reload(move |idx| create_folder_in(idx, &name).map(|_| ()));
        });
        dialog.present(Some(parent));
        field.grab_focus();
        field.select_region(0, -1);
    }

    fn ask_rename_folder(self: &Rc<Self>, parent: &gtk::Widget, old_name: &str) {
        let field = gtk::Entry::builder().text(old_name).activates_default(true).build();
        let dialog = adw::AlertDialog::builder().heading("Rename Folder").extra_child(&field).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("rename", "Rename")]);
        dialog.set_default_response(Some("rename"));
        dialog.set_close_response("cancel");
        let pm = self.clone();
        let old = old_name.to_string();
        let f = field.clone();
        dialog.connect_response(None, move |_, response| {
            let name = f.text().trim().to_string();
            if response == "rename" && !name.is_empty() {
                let old_c = old.clone();
                let new_c = name.clone();
                if pm.current_folder.borrow().as_deref() == Some(old.as_str()) {
                    *pm.current_folder.borrow_mut() = Some(new_c.clone());
                }
                pm.run_index_then_reload(move |idx| rename_folder_in(idx, &old_c, &new_c).map(|_| ()));
            }
        });
        dialog.present(Some(parent));
        field.grab_focus();
        field.select_region(0, -1);
    }

    fn ask_move_to_folder(self: &Rc<Self>, parent: &gtk::Widget, entry: &Entry) {
        let folders = self.index.borrow().folders.clone();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let dialog = adw::AlertDialog::builder()
            .heading("Move to Folder")
            .body(format!("Choose a folder for \"{}\".", entry.name))
            .extra_child(&gtk::ScrolledWindow::builder().child(&list).min_content_height(160).max_content_height(280).build())
            .build();
        dialog.add_responses(&[("cancel", "Cancel")]);
        dialog.set_close_response("cancel");

        let add_choice = |label_text: &str, target: Option<String>, current: bool| {
            let row = adw::ActionRow::builder().title(label_text).activatable(true).build();
            if current {
                row.add_suffix(&label("Current", &["tempo-dim", "tempo-small"]));
            }
            list.append(&row);
            let pm = self.clone();
            let path = entry.path.clone();
            let d = dialog.clone();
            row.connect_activated(move |_| {
                d.close();
                let target = target.clone();
                let path = path.clone();
                pm.run_index_then_reload(move |idx| {
                    move_entry_to_folder_in(idx, &path, target.as_deref());
                    Ok(())
                });
            });
        };

        add_choice("Projects (Root)", None, entry.folder.is_none());
        for f in folders {
            let is_cur = entry.folder.as_deref() == Some(f.as_str());
            add_choice(&f, Some(f.clone()), is_cur);
        }

        dialog.present(Some(parent));
    }

    /// List a project's rolling backups, newest first, to open as a copy or delete.
    fn show_backups(self: &Rc<Self>, entry: &Entry, on_open: impl Fn(PathBuf) + Clone + 'static) {
        let pm = self.clone();
        let entry = entry.clone();
        glib::MainContext::default().spawn_local(async move {
            let path = entry.path.clone();
            let found = gio::spawn_blocking(move || -> Result<Vec<(PathBuf, String, u64)>, String> {
                let id = tempo_project::read_project_id(&path).map_err(|e| e.to_string())?;
                let backups = tempo_project::list_backups(&backup_dir(id)).map_err(|e| e.to_string())?;
                Ok(backups
                    .into_iter()
                    .map(|b| {
                        let size = std::fs::metadata(&b.path).map(|m| m.len()).unwrap_or(0);
                        (b.path, b.taken.format("%Y-%m-%d %H.%M").to_string(), size)
                    })
                    .collect())
            })
            .await
            .unwrap_or_else(|_| Err("reading the backups stopped unexpectedly".into()));
            let backups = match found {
                Ok(b) => b,
                Err(e) => return pm.report(&format!("Could not read the backups of \"{}\": {e}", entry.name)),
            };
            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .css_classes(["boxed-list"])
                .build();
            let dialog = adw::AlertDialog::builder()
                .heading(format!("Backups of \"{}\"", entry.name))
                .body(if backups.is_empty() {
                    "There are no backups yet. One is kept every 10 minutes while you edit."
                } else {
                    "Opening a backup makes a new project next to the original. The original is not changed."
                })
                .extra_child(&gtk::ScrolledWindow::builder().child(&list).min_content_height(if backups.is_empty() { 0 } else { 260 }).build())
                .build();
            dialog.add_responses(&[("close", "Close")]);
            for (file, taken, size) in backups {
                let row = adw::ActionRow::builder()
                    .title(&taken)
                    .subtitle(format!("{:.1} MB", size as f64 / 1_048_576.0))
                    .build();
                let open = gtk::Button::builder().label("Open as Copy").valign(gtk::Align::Center).build();
                let delete = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .tooltip_text("Delete this backup")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();
                row.add_suffix(&open);
                row.add_suffix(&delete);
                list.append(&row);

                let (pm2, d, cb, original, file2, taken2) = (
                    pm.clone(),
                    dialog.clone(),
                    on_open.clone(),
                    entry.path.clone(),
                    file.clone(),
                    taken.clone(),
                );
                open.connect_clicked(move |_| {
                    let (pm, d, cb, original, file, taken) = (
                        pm2.clone(),
                        d.clone(),
                        cb.clone(),
                        original.clone(),
                        file2.clone(),
                        taken2.clone(),
                    );
                    glib::MainContext::default().spawn_local(async move {
                        let restored = gio::spawn_blocking(move || {
                            let (copy, name) = restore_backup(&file, &taken, &original)?;
                            let index = load_raw_index_at(&list_path());
                            let folder = index.entries.iter().find(|e| e.path == original).and_then(|e| e.folder.clone());
                            remember_in(&name, &copy, folder.as_deref());
                            Ok::<_, String>(copy)
                        });
                        match restored.await {
                            Ok(Ok(copy)) => {
                                d.close();
                                pm.reload();
                                cb(copy);
                            }
                            Ok(Err(e)) => pm.report(&format!("Could not open the backup: {e}")),
                            Err(_) => pm.report("Opening the backup stopped unexpectedly."),
                        }
                    });
                });
                let (pm2, list2, row2) = (pm.clone(), list.clone(), row.clone());
                delete.connect_clicked(move |_| {
                    let (pm, list, row, file) = (pm2.clone(), list2.clone(), row2.clone(), file.clone());
                    glib::MainContext::default().spawn_local(async move {
                        match gio::spawn_blocking(move || std::fs::remove_file(&file)).await {
                            Ok(Ok(())) => list.remove(&row),
                            Ok(Err(e)) => pm.report(&format!("Could not delete the backup: {e}")),
                            Err(_) => {}
                        }
                    });
                });
            }
            dialog.present(Some(&pm.root));
        });
    }

    fn ask_rename(self: &Rc<Self>, parent: &gtk::Widget, entry: &Entry) {
        let field = gtk::Entry::builder().text(&entry.name).activates_default(true).build();
        let dialog = adw::AlertDialog::builder().heading("Rename Project").extra_child(&field).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("rename", "Rename")]);
        dialog.set_default_response(Some("rename"));
        dialog.set_close_response("cancel");
        let pm = self.clone();
        let path = entry.path.clone();
        dialog.connect_response(None, move |_, response| {
            let name = field.text().trim().to_string();
            if response == "rename" && !name.is_empty() {
                pm.run_then_reload(path.clone(), move |p| rename_project(p, &name));
            }
        });
        dialog.present(Some(parent));
    }

    fn selected_item(&self) -> Option<ManagerItem> {
        self.selection
            .selected_item()
            .and_downcast::<glib::BoxedAnyObject>()
            .map(|o| o.borrow::<ManagerItem>().clone())
    }

    /// Re-read the index from disk on a worker thread.
    pub fn reload(self: &Rc<Self>) {
        let pm = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let index = gio::spawn_blocking(load_index).await.unwrap_or_default();
            // If the current folder was deleted externally, fall back to root.
            let missing_folder = pm
                .current_folder
                .borrow()
                .as_ref()
                .is_some_and(|cur| !index.folders.iter().any(|f| f == cur));
            if missing_folder {
                *pm.current_folder.borrow_mut() = None;
            }
            *pm.index.borrow_mut() = index;
            pm.show_filtered();
        });
    }

    fn update_stack_view(&self) {
        let index = self.index.borrow();
        let empty = index.entries.is_empty() && index.folders.is_empty();
        let view = if empty {
            "empty"
        } else if self.grid_btn.is_active() {
            "grid"
        } else {
            "list"
        };
        self.stack.set_visible_child_name(view);
    }

    fn show_filtered(&self) {
        let query = self.search.text().to_lowercase();
        let sort_mode = SortMode::from_index(self.sort_dropdown.selected());
        let current_folder = self.current_folder.borrow().clone();

        if let Some(ref folder) = current_folder {
            self.back_btn.set_visible(true);
            self.heading.set_text(&format!("Projects / {folder}"));
        } else {
            self.back_btn.set_visible(false);
            self.heading.set_text("Projects");
        }

        self.store.remove_all();
        let index = self.index.borrow();

        if current_folder.is_none() {
            let mut folders = index.folders.clone();
            sort_folders(&mut folders, sort_mode);
            for f in folders {
                if !query.is_empty() && !f.to_lowercase().contains(&query) {
                    continue;
                }
                let count = index.entries.iter().filter(|e| e.folder.as_deref() == Some(f.as_str())).count();
                self.store.append(&glib::BoxedAnyObject::new(ManagerItem::Folder { name: f, count }));
            }
        }

        let mut entries: Vec<Entry> = index
            .entries
            .iter()
            .filter(|e| match &current_folder {
                Some(f) => e.folder.as_deref() == Some(f.as_str()),
                None => e.folder.is_none(),
            })
            .filter(|e| query.is_empty() || e.name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        sort_entries(&mut entries, sort_mode);

        for e in entries {
            self.store.append(&glib::BoxedAnyObject::new(ManagerItem::Project(e)));
        }

        drop(index);
        self.update_stack_view();
    }
}

fn attach_cell_right_click(widget: &impl IsA<gtk::Widget>, item: &gtk::ListItem, on_menu: CellMenuFn) {
    let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
    let weak_item = item.downgrade();
    click.connect_pressed(move |gesture, _, x, y| {
        let (Some(item), Some(w)) = (weak_item.upgrade(), gesture.widget()) else { return };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(show) = on_menu.borrow().as_ref() {
            show(item.position(), &w, x, y);
        }
    });
    widget.add_controller(click);
}

fn card_factory(
    selection: &gtk::SingleSelection,
    thumb_width: Rc<Cell<i32>>,
    card_thumbs: Rc<RefCell<Vec<(glib::WeakRef<gtk::Overlay>, glib::WeakRef<gtk::Label>)>>>,
    on_menu: CellMenuFn,
) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let _selection = selection;
    let tw_setup = thumb_width.clone();
    factory.connect_setup(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let w = tw_setup.get();
        let h = (w * 9 / 16).max(1);
        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        let icon = gtk::Image::builder().icon_name("dev.tempo.Tempo-symbolic").pixel_size(40).build();
        let thumb = gtk::Overlay::builder()
            .child(&picture)
            .css_classes(["project-thumb"])
            .overflow(gtk::Overflow::Hidden)
            .halign(gtk::Align::Center)
            .build();
        thumb.set_size_request(w, h);
        thumb.add_overlay(&icon);

        let name = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars((w / 10).max(14))
            .css_classes(["project-name-label"])
            .build();
        let date = label("", &["tempo-small", "tempo-dim"]);
        let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        card.append(&thumb);
        card.append(&name);
        card.append(&date);

        let mut list = card_thumbs.borrow_mut();
        list.retain(|(t, _)| t.upgrade().is_some());
        list.push((thumb.downgrade(), name.downgrade()));

        attach_cell_right_click(&card, item, on_menu.clone());
        item.set_child(Some(&card));
    });

    let tw = thumb_width;
    factory.connect_bind(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let Some(boxed) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
        let mitem = boxed.borrow::<ManagerItem>();
        let Some(card) = item.child().and_downcast::<gtk::Box>() else { return };
        let Some(thumb) = card.first_child().and_downcast::<gtk::Overlay>() else { return };
        let name = thumb.next_sibling().and_downcast::<gtk::Label>();
        let date = card.last_child().and_downcast::<gtk::Label>();
        let picture = thumb.child().and_downcast::<gtk::Picture>();
        let icon = picture.as_ref().and_then(|p| p.next_sibling()).and_downcast::<gtk::Image>();

        let w = tw.get();
        thumb.set_size_request(w, (w * 9 / 16).max(1));
        if let Some(ref n) = name {
            n.set_max_width_chars((w / 10).max(14));
        }

        match &*mitem {
            ManagerItem::Folder { name: folder_name, count } => {
                if let Some(p) = &picture {
                    p.set_paintable(None::<&gdk::Texture>);
                }
                if let Some(i) = &icon {
                    i.set_icon_name(Some("folder-symbolic"));
                    i.set_visible(true);
                }
                if let Some(n) = &name {
                    n.set_text(folder_name);
                }
                if let Some(d) = &date {
                    d.set_text(&if *count == 1 {
                        "1 project".to_string()
                    } else {
                        format!("{count} projects")
                    });
                }
                card.set_tooltip_text(Some(&format!("Folder: {folder_name}")));
            }
            ManagerItem::Project(entry) => {
                let texture = if !entry.missing { entry.thumbnail.as_ref().map(thumb_texture) } else { None };
                let has_thumb = texture.is_some();
                if let Some(p) = &picture {
                    p.set_paintable(texture.as_ref());
                }
                if let Some(i) = &icon {
                    i.set_icon_name(Some(if entry.missing {
                        "dialog-warning-symbolic"
                    } else {
                        "dev.tempo.Tempo-symbolic"
                    }));
                    i.set_visible(!has_thumb);
                }
                if let Some(n) = &name {
                    n.set_text(&entry.name);
                }
                if let Some(d) = &date {
                    d.set_text(&if entry.missing {
                        "File not found".to_string()
                    } else {
                        relative_time(entry.modified)
                    });
                }
                card.set_tooltip_text(Some(&entry.path.to_string_lossy()));
            }
        }
    });
    factory
}

fn build_column_view(selection: &gtk::SingleSelection, on_menu: CellMenuFn) -> gtk::ColumnView {
    let view = gtk::ColumnView::builder()
        .model(selection)
        .show_column_separators(false)
        .show_row_separators(true)
        .vexpand(true)
        .hexpand(true)
        .build();

    let name_factory = gtk::SignalListItemFactory::new();
    let menu_cb = on_menu.clone();
    name_factory.connect_setup(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let icon = gtk::Image::builder().pixel_size(16).build();
        let lbl = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .xalign(0.0)
            .hexpand(true)
            .build();
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(6)
            .margin_end(6)
            .build();
        row.append(&icon);
        row.append(&lbl);
        attach_cell_right_click(&row, item, menu_cb.clone());
        item.set_child(Some(&row));
    });
    name_factory.connect_bind(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let Some(boxed) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
        let mitem = boxed.borrow::<ManagerItem>();
        let Some(row) = item.child().and_downcast::<gtk::Box>() else { return };
        let icon = row.first_child().and_downcast::<gtk::Image>();
        let lbl = row.last_child().and_downcast::<gtk::Label>();
        match &*mitem {
            ManagerItem::Folder { name, .. } => {
                if let Some(i) = icon {
                    i.set_icon_name(Some("folder-symbolic"));
                }
                if let Some(l) = lbl {
                    l.set_text(name);
                }
            }
            ManagerItem::Project(entry) => {
                if let Some(i) = icon {
                    i.set_icon_name(Some(if entry.missing {
                        "dialog-warning-symbolic"
                    } else {
                        "dev.tempo.Tempo-symbolic"
                    }));
                }
                if let Some(l) = lbl {
                    l.set_text(&entry.name);
                }
            }
        }
    });
    let name_col = gtk::ColumnViewColumn::builder()
        .title("Name")
        .factory(&name_factory)
        .expand(true)
        .resizable(true)
        .build();
    view.append_column(&name_col);

    let mod_factory = gtk::SignalListItemFactory::new();
    let menu_cb = on_menu.clone();
    mod_factory.connect_setup(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let lbl = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["tempo-dim", "tempo-small"])
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(6)
            .margin_end(6)
            .build();
        attach_cell_right_click(&lbl, item, menu_cb.clone());
        item.set_child(Some(&lbl));
    });
    mod_factory.connect_bind(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let Some(boxed) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
        let mitem = boxed.borrow::<ManagerItem>();
        let Some(lbl) = item.child().and_downcast::<gtk::Label>() else { return };
        match &*mitem {
            ManagerItem::Folder { count, .. } => {
                lbl.set_text(&if *count == 1 {
                    "1 project".to_string()
                } else {
                    format!("{count} projects")
                });
            }
            ManagerItem::Project(entry) => {
                lbl.set_text(&if entry.missing {
                    "File not found".to_string()
                } else {
                    relative_time(entry.modified)
                });
            }
        }
    });
    let mod_col = gtk::ColumnViewColumn::builder()
        .title("Modified")
        .factory(&mod_factory)
        .fixed_width(140)
        .resizable(true)
        .build();
    view.append_column(&mod_col);

    let path_factory = gtk::SignalListItemFactory::new();
    path_factory.connect_setup(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let lbl = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .hexpand(true)
            .css_classes(["tempo-dim", "tempo-small"])
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(6)
            .margin_end(6)
            .build();
        attach_cell_right_click(&lbl, item, on_menu.clone());
        item.set_child(Some(&lbl));
    });
    path_factory.connect_bind(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let Some(boxed) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
        let mitem = boxed.borrow::<ManagerItem>();
        let Some(lbl) = item.child().and_downcast::<gtk::Label>() else { return };
        match &*mitem {
            ManagerItem::Folder { .. } => lbl.set_text(""),
            ManagerItem::Project(entry) => lbl.set_text(&entry.path.to_string_lossy()),
        }
    });
    let path_col = gtk::ColumnViewColumn::builder()
        .title("Path")
        .factory(&path_factory)
        .expand(true)
        .resizable(true)
        .build();
    view.append_column(&path_col);

    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};
    use tempo_timeline::{MediaSource, MediaType, RationalFps};

    #[test]
    fn rename_and_duplicate_change_the_stored_name() {
        // Keep the project list of this test away from the user's own.
        let dir = std::env::temp_dir().join(format!("tempo-pm-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("One.tempo");
        tempo_project::create_new_project("One", 1920, 1080, RationalFps::FPS_30, &path).expect("create");

        let mut project = tempo_project::load_project(&path).expect("load");
        project.name = "Two".into();
        tempo_project::save_project(&project, &path).expect("save");
        assert_eq!(tempo_project::load_project(&path).expect("load").name, "Two");

        // The copy gets its own file and name; the original is untouched.
        let copy = dir.join("One copy.tempo");
        std::fs::copy(&path, &copy).expect("copy");
        let mut dup = tempo_project::load_project(&copy).expect("load copy");
        dup.name = format!("{} copy", dup.name);
        tempo_project::save_project(&dup, &copy).expect("save copy");
        assert_eq!(tempo_project::load_project(&copy).expect("load").name, "Two copy");
        assert_eq!(tempo_project::load_project(&path).expect("load").name, "Two");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restoring_a_backup_never_touches_the_original() {
        let dir = std::env::temp_dir().join(format!("tempo-backup-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let original = dir.join("Film.tempo");
        tempo_project::create_new_project("Film", 1920, 1080, RationalFps::FPS_30, &original).expect("create");
        let backup = dir.join("20261020-100000.tempo");
        std::fs::copy(&original, &backup).expect("copy");
        // Damage the original: the restore must still work and leave it as it is.
        std::fs::write(&original, b"not a database").expect("damage");
        let before = std::fs::read(&original).expect("read");

        let (copy, name) = super::restore_backup(&backup, "2026-10-20 10.00", &original).expect("restore");
        assert_eq!(copy, dir.join("Film (backup 2026-10-20 10.00).tempo"));
        assert_eq!(name, "Film (backup 2026-10-20 10.00)");
        let restored = tempo_project::load_project(&copy).expect("load");
        assert_eq!(restored.name, name);
        assert_ne!(restored.id, tempo_project::read_project_id(&backup).expect("id"), "the copy is a project of its own");
        assert_eq!(std::fs::read(&original).expect("read"), before);
        // A second restore of the same backup does not overwrite the first copy.
        assert!(super::restore_backup(&backup, "2026-10-20 10.00", &original).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_json_array_deserializes_and_upgrades_to_index() {
        let legacy_json = r#"[
            {"name": "Film A", "path": "/tmp/a.tempo"},
            {"name": "Film B", "path": "/tmp/b.tempo"}
        ]"#;
        let index = parse_index(legacy_json);
        assert!(index.folders.is_empty());
        assert_eq!(index.entries.len(), 2);
        assert_eq!(index.entries[0].name, "Film A");
        assert_eq!(index.entries[0].folder, None);

        let serialized = serde_json::to_string_pretty(&index).expect("serialize");
        let reparsed = parse_index(&serialized);
        assert_eq!(reparsed, index);
    }

    #[test]
    fn folders_crud_move_and_delete_return_entries_to_root() {
        let mut index = ProjectIndex {
            folders: Vec::new(),
            entries: vec![
                Entry {
                    name: "Alpha".into(),
                    path: PathBuf::from("/tmp/alpha.tempo"),
                    folder: None,
                    modified: None,
                    missing: false,
                    thumbnail: None,
                },
                Entry {
                    name: "Beta".into(),
                    path: PathBuf::from("/tmp/beta.tempo"),
                    folder: None,
                    modified: None,
                    missing: false,
                    thumbnail: None,
                },
            ],
        };

        create_folder_in(&mut index, "Client").expect("create folder");
        assert_eq!(index.folders, vec!["Client"]);
        assert!(create_folder_in(&mut index, "client").is_err(), "duplicate folder rejected case-insensitively");

        move_entry_to_folder_in(&mut index, Path::new("/tmp/alpha.tempo"), Some("Client"));
        assert_eq!(index.entries[0].folder.as_deref(), Some("Client"));
        assert_eq!(index.entries[1].folder, None);

        rename_folder_in(&mut index, "Client", "Archive").expect("rename folder");
        assert_eq!(index.folders, vec!["Archive"]);
        assert_eq!(index.entries[0].folder.as_deref(), Some("Archive"));

        delete_folder_in(&mut index, "Archive");
        assert!(index.folders.is_empty());
        assert_eq!(index.entries[0].folder, None);
    }

    #[test]
    fn sort_orders_entries_by_date_and_name() {
        let mut entries = vec![
            Entry {
                name: "charlie".into(),
                path: PathBuf::from("/tmp/c.tempo"),
                folder: None,
                modified: Some(UNIX_EPOCH + Duration::from_secs(10)),
                missing: false,
                thumbnail: None,
            },
            Entry {
                name: "Alpha".into(),
                path: PathBuf::from("/tmp/a.tempo"),
                folder: None,
                modified: Some(UNIX_EPOCH + Duration::from_secs(30)),
                missing: false,
                thumbnail: None,
            },
            Entry {
                name: "bravo".into(),
                path: PathBuf::from("/tmp/b.tempo"),
                folder: None,
                modified: Some(UNIX_EPOCH + Duration::from_secs(20)),
                missing: false,
                thumbnail: None,
            },
        ];

        sort_entries(&mut entries, SortMode::DateModified);
        assert_eq!(entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["Alpha", "bravo", "charlie"]);

        sort_entries(&mut entries, SortMode::NameAsc);
        assert_eq!(entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["Alpha", "bravo", "charlie"]);

        sort_entries(&mut entries, SortMode::NameDesc);
        assert_eq!(entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["charlie", "bravo", "Alpha"]);
    }

    #[test]
    fn duplicate_name_and_default_name_are_case_insensitive() {
        let existing = vec!["OC 3".to_string(), "Untitled Project".to_string(), "untitled project 2".to_string()];
        assert!(is_duplicate_name(&existing, "oc 3"));
        assert!(is_duplicate_name(&existing, "  OC 3 "));
        assert!(!is_duplicate_name(&existing, "OC 4"));
        assert!(!is_duplicate_name(&existing, "   "));

        assert_eq!(next_default_project_name(&[]), "Untitled Project");
        assert_eq!(next_default_project_name(&["Untitled Project".into()]), "Untitled Project 2");
        assert_eq!(next_default_project_name(&existing), "Untitled Project 3");
    }

    fn test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tempo-pm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test dir");
        dir
    }

    fn entry(name: &str, path: &str, folder: Option<&str>) -> Entry {
        Entry {
            name: name.into(),
            path: PathBuf::from(path),
            folder: folder.map(str::to_string),
            modified: None,
            missing: false,
            thumbnail: None,
        }
    }

    #[test]
    fn export_onto_the_projects_own_file_is_refused_and_keeps_it() {
        let dir = test_dir("export");
        let src = dir.join("Film.tempo");
        std::fs::write(&src, vec![7u8; 4096]).expect("write");
        let link = dir.join("link.tempo");
        std::fs::hard_link(&src, &link).expect("hard link");

        assert!(export_project_file(&src, &src).is_err());
        assert!(export_project_file(&src, &link).is_err());
        assert_eq!(std::fs::metadata(&src).expect("meta").len(), 4096, "the project is not emptied");

        let out = dir.join("out").join("Film.tempo");
        export_project_file(&src, &out).expect("export elsewhere");
        assert_eq!(std::fs::read(&out).expect("read"), vec![7u8; 4096]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tempo_extension_is_appended_not_swapped() {
        assert_eq!(with_tempo_extension("/a/My.Film".into()), PathBuf::from("/a/My.Film.tempo"));
        assert_eq!(with_tempo_extension("/a/Film".into()), PathBuf::from("/a/Film.tempo"));
        assert_eq!(with_tempo_extension("/a/Film.tempo".into()), PathBuf::from("/a/Film.tempo"));
    }

    #[test]
    fn new_projects_go_into_the_given_folder_and_known_ones_keep_theirs() {
        let dir = test_dir("remember");
        let list = dir.join("projects.json");
        let index = ProjectIndex {
            folders: vec!["Client Work".into(), "Archive".into()],
            entries: vec![entry("Old", "/tmp/old.tempo", Some("Archive"))],
        };
        save_index_at(&list, &index).expect("save");

        remember_at(&list, "New", Path::new("/tmp/new.tempo"), Some("Client Work"));
        remember_at(&list, "Old", Path::new("/tmp/old.tempo"), Some("Client Work"));
        let index = load_raw_index_at(&list);
        let folder_of = |p: &str| index.entries.iter().find(|e| e.path == Path::new(p)).and_then(|e| e.folder.clone());
        assert_eq!(folder_of("/tmp/new.tempo").as_deref(), Some("Client Work"));
        assert_eq!(folder_of("/tmp/old.tempo").as_deref(), Some("Archive"), "an existing entry keeps its folder");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_as_gives_a_new_file_its_own_id_and_checks_the_name() {
        let dir = test_dir("save-as");
        let list = dir.join("projects.json");
        let index = ProjectIndex {
            folders: Vec::new(),
            entries: vec![entry("Film", "/p/Film.tempo", None), entry("Cut/1", "/p/Cut_1.tempo", None)],
        };
        save_index_at(&list, &index).expect("save");
        let id = uuid::Uuid::new_v4();
        let here = Some(Path::new("/p/Film.tempo"));

        // A new file: new id, name from the typed file name.
        let (name, new_id) = save_as_identity_at(&list, "Film", id, here, Path::new("/q/Film v2.tempo")).expect("ok");
        assert_eq!(name, "Film v2");
        assert_ne!(new_id, id);
        // Same file: same id, so its own backups stay its own.
        let (name, same_id) = save_as_identity_at(&list, "Film", id, here, Path::new("/p/Film.tempo")).expect("ok");
        assert_eq!((name.as_str(), same_id), ("Film", id));
        // The suggested name in another folder would be a second "Film".
        assert!(save_as_identity_at(&list, "Film", id, here, Path::new("/q/film.tempo")).is_err());
        // Keeping the suggested (made safe) file name is not a rename.
        let cut = Some(Path::new("/p/Cut_1.tempo"));
        let (name, _) = save_as_identity_at(&list, "Cut/1", id, cut, Path::new("/p/Cut_1.tempo")).expect("ok");
        assert_eq!(name, "Cut/1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_thumbnail_saves_and_loads_le_rgba() {
        let dir = std::env::temp_dir().join(format!("tempo-thumb-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut project = Project::new("Thumb", 1920, 1080, RationalFps::FPS_30);
        let mut src = MediaSource::new(PathBuf::from("/tmp/clip.mp4"), MediaType::Video, 1_000_000);
        src.import_order = 1;
        src.thumbnail_data = Some(vec![0xAA; 160 * 90 * 4]);
        project.sources.insert(src.id, src);

        save_project_thumbnail_in(&project, &dir);
        let loaded = load_thumbnail_in(&dir, project.id).expect("thumbnail should load");
        assert_eq!(loaded.width, 160);
        assert_eq!(loaded.height, 90);
        assert_eq!(loaded.rgba.len(), 160 * 90 * 4);
        assert!(loaded.rgba.iter().all(|&b| b == 0xAA));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
