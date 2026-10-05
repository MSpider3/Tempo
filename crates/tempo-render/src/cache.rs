use std::sync::Arc;
use indexmap::IndexMap;
use uuid::Uuid;
use tempo_media::VideoFrame;
use tempo_timeline::types::{frame_to_us, us_to_frame, RationalFps};

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct FrameCacheKey {
    pub source_id: Uuid,
    pub pts_us: i64,
}

pub struct CachedFrame {
    pub texture: Arc<wgpu::Texture>,
    pub view: Arc<wgpu::TextureView>,
    pub size_bytes: usize,
    pub width: u32,
    pub height: u32,
}

pub struct FrameCache {
    entries: IndexMap<FrameCacheKey, CachedFrame>,
    current_bytes: usize,
    max_bytes: usize,
}

impl FrameCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            entries: IndexMap::new(),
            current_bytes: 0,
            max_bytes,
        }
    }

    pub fn quantize(pts_us: i64, fps: RationalFps) -> i64 {
        let frame = us_to_frame(pts_us, fps);
        frame_to_us(frame, fps)
    }

    pub fn get(&mut self, key: &FrameCacheKey) -> Option<Arc<wgpu::TextureView>> {
        if self.entries.contains_key(key) {
            let idx = self.entries.get_index_of(key).unwrap();
            let last_idx = self.entries.len() - 1;
            self.entries.move_index(idx, last_idx);
            self.entries.get(key).map(|cf| cf.view.clone())
        } else {
            None
        }
    }

    pub fn insert(
        &mut self,
        key: FrameCacheKey,
        frame: &VideoFrame,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let width = frame.width;
        let height = frame.height;
        let size_bytes = (width * height * 4) as usize;

        // If key already exists, remove it first
        if let Some(old) = self.entries.shift_remove(&key) {
            self.current_bytes = self.current_bytes.saturating_sub(old.size_bytes);
        }

        // Evict LRU entries until under budget
        while self.current_bytes + size_bytes > self.max_bytes && !self.entries.is_empty() {
            let (_, evicted) = self.entries.shift_remove_index(0).unwrap();
            self.current_bytes = self.current_bytes.saturating_sub(evicted.size_bytes);
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cached_video_frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.entries.insert(
            key,
            CachedFrame {
                texture: Arc::new(texture),
                view: Arc::new(view),
                size_bytes,
                width,
                height,
            },
        );
        self.current_bytes += size_bytes;
    }

    pub fn evict_source(&mut self, source_id: Uuid) {
        let keys_to_remove: Vec<FrameCacheKey> = self
            .entries
            .keys()
            .filter(|k| k.source_id == source_id)
            .cloned()
            .collect();

        for k in keys_to_remove {
            if let Some(cf) = self.entries.shift_remove(&k) {
                self.current_bytes = self.current_bytes.saturating_sub(cf.size_bytes);
            }
        }
    }

    pub fn current_bytes(&self) -> usize {
        self.current_bytes
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.current_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_media::PixelFormat;

    #[test]
    fn test_frame_cache_eviction() {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .expect("Failed to get wgpu adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("test_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .expect("Failed to get wgpu device");

        // Frame size: 100 x 100 x 4 = 40,000 bytes
        let frame_size = 100 * 100 * 4;
        // Limit cache to hold exactly 2 frames: 80,000 bytes
        let mut cache = FrameCache::new(frame_size * 2);

        let source_id = Uuid::new_v4();

        let f1 = VideoFrame {
            width: 100,
            height: 100,
            format: PixelFormat::Rgba,
            data: vec![10u8; frame_size],
            pts_us: 0,
        };
        let f2 = VideoFrame {
            width: 100,
            height: 100,
            format: PixelFormat::Rgba,
            data: vec![20u8; frame_size],
            pts_us: 33333,
        };
        let f3 = VideoFrame {
            width: 100,
            height: 100,
            format: PixelFormat::Rgba,
            data: vec![30u8; frame_size],
            pts_us: 66666,
        };

        let k1 = FrameCacheKey { source_id, pts_us: 0 };
        let k2 = FrameCacheKey { source_id, pts_us: 33333 };
        let k3 = FrameCacheKey { source_id, pts_us: 66666 };

        cache.insert(k1.clone(), &f1, &device, &queue);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.current_bytes(), frame_size);

        cache.insert(k2.clone(), &f2, &device, &queue);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.current_bytes(), frame_size * 2);

        // Access k1 to make k2 the LRU item
        let _ = cache.get(&k1);

        // Insert f3: k2 should be evicted!
        cache.insert(k3.clone(), &f3, &device, &queue);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.current_bytes(), frame_size * 2);

        assert!(cache.get(&k1).is_some());
        assert!(cache.get(&k2).is_none()); // k2 was evicted!
        assert!(cache.get(&k3).is_some());
    }
}
