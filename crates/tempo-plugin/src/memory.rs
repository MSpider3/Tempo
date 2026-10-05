use wasmtime::Caller;

pub fn read_wasm_string<T>(caller: &mut Caller<'_, T>, ptr: i32, len: i32) -> Result<String, String> {
    if ptr < 0 || len < 0 {
        return Err("Invalid pointer or length".to_string());
    }
    let memory = match caller.get_export("memory") {
        Some(wasmtime::Extern::Memory(m)) => m,
        _ => return Err("WASM module has no exported memory".to_string()),
    };
    let data = memory.data(caller);
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or_else(|| "Pointer overflow".to_string())?;
    if end > data.len() {
        return Err("Memory access out of bounds".to_string());
    }
    let slice = &data[start..end];
    String::from_utf8(slice.to_vec()).map_err(|e| format!("Invalid UTF-8: {}", e))
}

pub fn write_wasm_bytes<T>(caller: &mut Caller<'_, T>, ptr: i32, bytes: &[u8]) -> Result<(), String> {
    if ptr < 0 {
        return Err("Invalid destination pointer".to_string());
    }
    let memory = match caller.get_export("memory") {
        Some(wasmtime::Extern::Memory(m)) => m,
        _ => return Err("WASM module has no exported memory".to_string()),
    };
    let start = ptr as usize;
    let end = start
        .checked_add(bytes.len())
        .ok_or_else(|| "Pointer overflow".to_string())?;
    let data = memory.data_mut(caller);
    if end > data.len() {
        return Err("Memory access out of bounds".to_string());
    }
    data[start..end].copy_from_slice(bytes);
    Ok(())
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WasmClipInfo {
    pub clip_id: [u8; 16],
    pub source_id: [u8; 16],
    pub timeline_in_us: i64,
    pub timeline_out_us: i64,
    pub source_in_us: i64,
    pub source_out_us: i64,
    pub clip_type: i32, // 0=video, 1=audio, 2=image, 3=title
}

impl WasmClipInfo {
    pub fn to_bytes(&self) -> [u8; 68] {
        let mut buf = [0u8; 68];
        buf[0..16].copy_from_slice(&self.clip_id);
        buf[16..32].copy_from_slice(&self.source_id);
        buf[32..40].copy_from_slice(&self.timeline_in_us.to_le_bytes());
        buf[40..48].copy_from_slice(&self.timeline_out_us.to_le_bytes());
        buf[48..56].copy_from_slice(&self.source_in_us.to_le_bytes());
        buf[56..64].copy_from_slice(&self.source_out_us.to_le_bytes());
        buf[64..68].copy_from_slice(&self.clip_type.to_le_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < 68 {
            return None;
        }
        let mut clip_id = [0u8; 16];
        let mut source_id = [0u8; 16];
        clip_id.copy_from_slice(&buf[0..16]);
        source_id.copy_from_slice(&buf[16..32]);
        let timeline_in_us = i64::from_le_bytes(buf[32..40].try_into().unwrap());
        let timeline_out_us = i64::from_le_bytes(buf[40..48].try_into().unwrap());
        let source_in_us = i64::from_le_bytes(buf[48..56].try_into().unwrap());
        let source_out_us = i64::from_le_bytes(buf[56..64].try_into().unwrap());
        let clip_type = i32::from_le_bytes(buf[64..68].try_into().unwrap());
        Some(Self {
            clip_id,
            source_id,
            timeline_in_us,
            timeline_out_us,
            source_in_us,
            source_out_us,
            clip_type,
        })
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WasmMediaInfo {
    pub struct_size: i32, // 40
    pub media_type: i32,  // 0=video, 1=audio, 2=image
    pub duration_us: i64,
    pub width: i32,
    pub height: i32,
    pub fps_num: i32,
    pub fps_den: i32,
    pub sample_rate: i32,
    pub channels: i32,
}

impl WasmMediaInfo {
    pub fn to_bytes(&self) -> [u8; 40] {
        let mut buf = [0u8; 40];
        buf[0..4].copy_from_slice(&self.struct_size.to_le_bytes());
        buf[4..8].copy_from_slice(&self.media_type.to_le_bytes());
        buf[8..16].copy_from_slice(&self.duration_us.to_le_bytes());
        buf[16..20].copy_from_slice(&self.width.to_le_bytes());
        buf[20..24].copy_from_slice(&self.height.to_le_bytes());
        buf[24..28].copy_from_slice(&self.fps_num.to_le_bytes());
        buf[28..32].copy_from_slice(&self.fps_den.to_le_bytes());
        buf[32..36].copy_from_slice(&self.sample_rate.to_le_bytes());
        buf[36..40].copy_from_slice(&self.channels.to_le_bytes());
        buf
    }
}
