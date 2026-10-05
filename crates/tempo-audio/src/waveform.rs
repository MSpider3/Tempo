use rusqlite::params;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinMaxSample {
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WaveformMipMap {
    pub base_samples_per_block: usize,
    pub level_1x: Vec<MinMaxSample>,
    pub level_4x: Vec<MinMaxSample>,
    pub level_16x: Vec<MinMaxSample>,
    pub level_64x: Vec<MinMaxSample>,
}

const MAGIC: &[u8; 4] = b"TWAV";
const VERSION: u32 = 1;

impl WaveformMipMap {
    pub fn generate(samples: &[f32], base_samples_per_block: usize) -> Self {
        let base = base_samples_per_block.max(1);

        // 1x level
        let mut level_1x = Vec::with_capacity((samples.len() + base - 1) / base);
        for chunk in samples.chunks(base) {
            let mut min = f32::INFINITY;
            let mut max = f32::NEG_INFINITY;
            for &s in chunk {
                if s < min {
                    min = s;
                }
                if s > max {
                    max = s;
                }
            }
            if min.is_infinite() {
                min = 0.0;
            }
            if max.is_infinite() {
                max = 0.0;
            }
            level_1x.push(MinMaxSample { min, max });
        }

        // Downsample helper: aggregates 4 child blocks into 1
        fn downsample_4x(source: &[MinMaxSample]) -> Vec<MinMaxSample> {
            let mut result = Vec::with_capacity((source.len() + 3) / 4);
            for chunk in source.chunks(4) {
                let mut min = f32::INFINITY;
                let mut max = f32::NEG_INFINITY;
                for item in chunk {
                    if item.min < min {
                        min = item.min;
                    }
                    if item.max > max {
                        max = item.max;
                    }
                }
                result.push(MinMaxSample { min, max });
            }
            result
        }

        let level_4x = downsample_4x(&level_1x);
        let level_16x = downsample_4x(&level_4x);
        let level_64x = downsample_4x(&level_16x);

        Self {
            base_samples_per_block: base,
            level_1x,
            level_4x,
            level_16x,
            level_64x,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let total_samples = self.level_1x.len() + self.level_4x.len() + self.level_16x.len() + self.level_64x.len();
        let mut bytes = Vec::with_capacity(24 + total_samples * 8);

        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&(self.base_samples_per_block as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.level_1x.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.level_4x.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.level_16x.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(self.level_64x.len() as u32).to_le_bytes());

        let write_level = |bytes: &mut Vec<u8>, level: &[MinMaxSample]| {
            for sample in level {
                bytes.extend_from_slice(&sample.min.to_le_bytes());
                bytes.extend_from_slice(&sample.max.to_le_bytes());
            }
        };

        write_level(&mut bytes, &self.level_1x);
        write_level(&mut bytes, &self.level_4x);
        write_level(&mut bytes, &self.level_16x);
        write_level(&mut bytes, &self.level_64x);

        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 28 || &bytes[0..4] != MAGIC {
            return None;
        }

        let version = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
        if version != VERSION {
            return None;
        }

        let base_samples_per_block = u32::from_le_bytes(bytes[8..12].try_into().ok()?) as usize;
        let len_1x = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
        let len_4x = u32::from_le_bytes(bytes[16..20].try_into().ok()?) as usize;
        let len_16x = u32::from_le_bytes(bytes[20..24].try_into().ok()?) as usize;
        let len_64x = u32::from_le_bytes(bytes[24..28].try_into().ok()?) as usize;

        let total_samples = len_1x + len_4x + len_16x + len_64x;
        if bytes.len() < 28 + total_samples * 8 {
            return None;
        }

        let mut offset = 28;
        let read_level = |offset: &mut usize, len: usize| -> Vec<MinMaxSample> {
            let mut level = Vec::with_capacity(len);
            for _ in 0..len {
                let min = f32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap());
                let max = f32::from_le_bytes(bytes[*offset + 4..*offset + 8].try_into().unwrap());
                *offset += 8;
                level.push(MinMaxSample { min, max });
            }
            level
        };

        let level_1x = read_level(&mut offset, len_1x);
        let level_4x = read_level(&mut offset, len_4x);
        let level_16x = read_level(&mut offset, len_16x);
        let level_64x = read_level(&mut offset, len_64x);

        Some(Self {
            base_samples_per_block,
            level_1x,
            level_4x,
            level_16x,
            level_64x,
        })
    }

    pub fn save_to_sqlite(&self, conn: &rusqlite::Connection, source_id: &str, mtime: i64) -> rusqlite::Result<()> {
        let blob = self.to_bytes();
        conn.execute(
            "INSERT OR REPLACE INTO waveform_cache (source_id, block_count, data, source_mtime) VALUES (?1, ?2, ?3, ?4)",
            params![source_id, self.level_1x.len() as i64, blob, mtime],
        )?;
        Ok(())
    }

    pub fn load_from_sqlite(conn: &rusqlite::Connection, source_id: &str) -> rusqlite::Result<Option<Self>> {
        let mut stmt = conn.prepare("SELECT data FROM waveform_cache WHERE source_id = ?1")?;
        let mut rows = stmt.query(params![source_id])?;

        if let Some(row) = rows.next()? {
            let blob: Vec<u8> = row.get(0)?;
            Ok(Self::from_bytes(&blob))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_waveform_generation() {
        // 1. Generate 10-second audio clip at 48 kHz
        let sample_rate = 48000;
        let duration_secs = 10;
        let num_samples = sample_rate * duration_secs; // 480,000 samples

        let mut samples = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let val = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.8;
            samples.push(val);
        }

        // 2. Measure generation time and assert < 50ms
        let start_time = Instant::now();
        let waveform = WaveformMipMap::generate(&samples, 256);
        let elapsed = start_time.elapsed();

        assert!(
            elapsed.as_millis() < 50,
            "Waveform generation for 10s audio must be < 50ms, took {:?}",
            elapsed
        );

        // 3. Assert min/max pairs generated at multiple mip levels (1x, 4x, 16x, 64x)
        assert!(!waveform.level_1x.is_empty(), "Level 1x must not be empty");
        assert!(!waveform.level_4x.is_empty(), "Level 4x must not be empty");
        assert!(!waveform.level_16x.is_empty(), "Level 16x must not be empty");
        assert!(!waveform.level_64x.is_empty(), "Level 64x must not be empty");

        // Check relative sizes match the pyramid downsampling
        assert_eq!(waveform.level_1x.len(), (num_samples + 255) / 256);
        assert_eq!(waveform.level_4x.len(), (waveform.level_1x.len() + 3) / 4);
        assert_eq!(waveform.level_16x.len(), (waveform.level_4x.len() + 3) / 4);
        assert_eq!(waveform.level_64x.len(), (waveform.level_16x.len() + 3) / 4);

        // 4. Assert SQLite cache store and retrieve matches within epsilon
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS waveform_cache (
                source_id       TEXT PRIMARY KEY,
                block_count     INTEGER NOT NULL,
                data            BLOB NOT NULL,
                source_mtime    INTEGER NOT NULL
            );",
            [],
        )
        .unwrap();

        let source_id = "test-audio-source-123";
        let mtime = 1700000000;

        waveform.save_to_sqlite(&conn, source_id, mtime).expect("save to SQLite failed");

        let loaded = WaveformMipMap::load_from_sqlite(&conn, source_id)
            .expect("load from SQLite failed")
            .expect("waveform not found in SQLite");

        assert_eq!(loaded.base_samples_per_block, waveform.base_samples_per_block);
        assert_eq!(loaded.level_1x.len(), waveform.level_1x.len());
        assert_eq!(loaded.level_4x.len(), waveform.level_4x.len());
        assert_eq!(loaded.level_16x.len(), waveform.level_16x.len());
        assert_eq!(loaded.level_64x.len(), waveform.level_64x.len());

        let epsilon = 1e-5;
        for (orig, load) in waveform.level_1x.iter().zip(&loaded.level_1x) {
            assert!((orig.min - load.min).abs() < epsilon);
            assert!((orig.max - load.max).abs() < epsilon);
        }
        for (orig, load) in waveform.level_4x.iter().zip(&loaded.level_4x) {
            assert!((orig.min - load.min).abs() < epsilon);
            assert!((orig.max - load.max).abs() < epsilon);
        }
        for (orig, load) in waveform.level_16x.iter().zip(&loaded.level_16x) {
            assert!((orig.min - load.min).abs() < epsilon);
            assert!((orig.max - load.max).abs() < epsilon);
        }
        for (orig, load) in waveform.level_64x.iter().zip(&loaded.level_64x) {
            assert!((orig.min - load.min).abs() < epsilon);
            assert!((orig.max - load.max).abs() < epsilon);
        }
    }
}
