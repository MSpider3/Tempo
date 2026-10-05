use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const SAMPLE_RATE: u32 = 48000;
pub const CHANNELS: u8 = 2;
pub const MAX_AV_DRIFT_US: i64 = 40_000; // 40ms

#[derive(Debug, Clone)]
pub struct AudioEngineState {
    pub position_us: i64,
    pub is_playing: bool,
    pub speed: f32,
}

pub struct AudioEngine {
    position_us: Arc<AtomicI64>,
    is_playing: Arc<AtomicBool>,
    speed_percent: Arc<AtomicI32>, // 100 = 1.0x, -200 = -2.0x
    stop_signal: Arc<AtomicBool>,
    clock_thread: Option<JoinHandle<()>>,
}

impl AudioEngine {
    pub fn new() -> Self {
        let position_us = Arc::new(AtomicI64::new(0));
        let is_playing = Arc::new(AtomicBool::new(false));
        let speed_percent = Arc::new(AtomicI32::new(100));
        let stop_signal = Arc::new(AtomicBool::new(false));

        // Spawn high-precision audio/clock synchronization thread
        let pos_clone = position_us.clone();
        let playing_clone = is_playing.clone();
        let speed_clone = speed_percent.clone();
        let stop_clone = stop_signal.clone();

        let clock_thread = thread::Builder::new()
            .name("tempo-audio-clock".into())
            .spawn(move || {
                let tick_interval = Duration::from_millis(5); // 200 Hz clock resolution
                let mut last_instant = Instant::now();

                while !stop_clone.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    let elapsed = now.duration_since(last_instant);
                    last_instant = now;

                    if playing_clone.load(Ordering::Acquire) {
                        let elapsed_us = elapsed.as_micros() as i64;
                        let speed = speed_clone.load(Ordering::Relaxed) as f64 / 100.0;
                        let delta_us = (elapsed_us as f64 * speed) as i64;

                        let cur = pos_clone.load(Ordering::Relaxed);
                        let next = (cur + delta_us).max(0);
                        pos_clone.store(next, Ordering::Release);
                    }

                    thread::sleep(tick_interval);
                }
            })
            .ok();

        Self {
            position_us,
            is_playing,
            speed_percent,
            stop_signal,
            clock_thread,
        }
    }

    pub fn play(&self) {
        self.is_playing.store(true, Ordering::Release);
    }

    pub fn pause(&self) {
        self.is_playing.store(false, Ordering::Release);
    }

    pub fn toggle_playback(&self) -> bool {
        let was_playing = self.is_playing.load(Ordering::Acquire);
        let new_state = !was_playing;
        self.is_playing.store(new_state, Ordering::Release);
        new_state
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Acquire)
    }

    pub fn position_us(&self) -> i64 {
        self.position_us.load(Ordering::Acquire)
    }

    pub fn seek(&self, pos_us: i64) {
        self.position_us.store(pos_us.max(0), Ordering::Release);
    }

    pub fn speed(&self) -> f32 {
        self.speed_percent.load(Ordering::Acquire) as f32 / 100.0
    }

    pub fn set_speed(&self, speed: f32) {
        let pct = (speed * 100.0).round() as i32;
        self.speed_percent.store(pct, Ordering::Release);
    }

    /// J/K/L speed ramping:
    /// L: forward (1x -> 2x -> 4x -> 8x)
    /// K: pause (0x)
    /// J: reverse (-1x -> -2x -> -4x -> -8x)
    pub fn ramp_forward(&self) {
        let cur_speed = self.speed();
        if !self.is_playing() || cur_speed <= 0.0 {
            self.set_speed(1.0);
            self.play();
        } else if (cur_speed - 1.0).abs() < 0.1 {
            self.set_speed(2.0);
        } else if (cur_speed - 2.0).abs() < 0.1 {
            self.set_speed(4.0);
        } else if (cur_speed - 4.0).abs() < 0.1 {
            self.set_speed(8.0);
        } else {
            self.set_speed(8.0);
        }
    }

    pub fn ramp_reverse(&self) {
        let cur_speed = self.speed();
        if !self.is_playing() || cur_speed >= 0.0 {
            self.set_speed(-1.0);
            self.play();
        } else if (cur_speed - -1.0).abs() < 0.1 {
            self.set_speed(-2.0);
        } else if (cur_speed - -2.0).abs() < 0.1 {
            self.set_speed(-4.0);
        } else if (cur_speed - -4.0).abs() < 0.1 {
            self.set_speed(-8.0);
        } else {
            self.set_speed(-8.0);
        }
    }

    pub fn get_atomic_playhead(&self) -> Arc<AtomicI64> {
        self.position_us.clone()
    }

    pub fn state(&self) -> AudioEngineState {
        AudioEngineState {
            position_us: self.position_us(),
            is_playing: self.is_playing(),
            speed: self.speed(),
        }
    }

    /// Calculate A/V drift in microseconds (audio_pos_us - video_pts_us).
    /// Positive value means audio is ahead (video is lagging).
    pub fn calculate_drift_us(audio_pos_us: i64, video_pts_us: i64) -> i64 {
        audio_pos_us - video_pts_us
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop_signal.store(true, Ordering::Relaxed);
        if let Some(handle) = self.clock_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_engine_transport() {
        let engine = AudioEngine::new();
        assert_eq!(engine.is_playing(), false);
        assert_eq!(engine.position_us(), 0);

        engine.seek(1_000_000);
        assert_eq!(engine.position_us(), 1_000_000);

        engine.play();
        assert_eq!(engine.is_playing(), true);

        // Let it run for 100ms
        thread::sleep(Duration::from_millis(100));
        let pos = engine.position_us();
        assert!(pos > 1_050_000); // advanced by ~100,000 us

        engine.pause();
        assert_eq!(engine.is_playing(), false);
        let paused_pos = engine.position_us();
        thread::sleep(Duration::from_millis(50));
        // Position should not advance while paused
        assert_eq!(engine.position_us(), paused_pos);
    }

    #[test]
    fn test_jkl_speed_ramping() {
        let engine = AudioEngine::new();
        assert_eq!(engine.is_playing(), false);

        // Press L: starts 1x
        engine.ramp_forward();
        assert_eq!(engine.is_playing(), true);
        assert!((engine.speed() - 1.0).abs() < 0.01);

        // Press L again: 2x
        engine.ramp_forward();
        assert!((engine.speed() - 2.0).abs() < 0.01);

        // Press L again: 4x
        engine.ramp_forward();
        assert!((engine.speed() - 4.0).abs() < 0.01);

        // Press L again: 8x
        engine.ramp_forward();
        assert!((engine.speed() - 8.0).abs() < 0.01);

        // Press K: pause
        engine.pause();
        assert_eq!(engine.is_playing(), false);

        // Press J: reverse -1x
        engine.ramp_reverse();
        assert_eq!(engine.is_playing(), true);
        assert!((engine.speed() - -1.0).abs() < 0.01);

        // Press J again: -2x
        engine.ramp_reverse();
        assert!((engine.speed() - -2.0).abs() < 0.01);
    }

    #[test]
    fn test_av_drift_calculation() {
        // Video at 1.0s, Audio at 1.03s -> drift = +30ms (acceptable)
        let drift = AudioEngine::calculate_drift_us(1_030_000, 1_000_000);
        assert_eq!(drift, 30_000);
        assert!(drift.abs() < MAX_AV_DRIFT_US);

        // Video at 1.0s, Audio at 1.05s -> drift = +50ms (exceeds 40ms threshold)
        let drift_lag = AudioEngine::calculate_drift_us(1_050_000, 1_000_000);
        assert_eq!(drift_lag, 50_000);
        assert!(drift_lag > MAX_AV_DRIFT_US);
    }
}
