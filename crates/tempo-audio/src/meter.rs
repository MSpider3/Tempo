#[derive(Debug, Clone, PartialEq)]
pub struct MeterReading {
    pub peak_db: f32,
    pub rms_db: f32,
    pub peak_hold_db: f32,
    pub is_clipping: bool,
}

pub struct AudioMeter {
    pub peak_hold_db: f32,
    pub is_clipping: bool,
}

impl AudioMeter {
    pub fn new() -> Self {
        Self {
            peak_hold_db: f32::NEG_INFINITY,
            is_clipping: false,
        }
    }

    pub fn process_samples(&mut self, samples: &[f32]) -> MeterReading {
        if samples.is_empty() {
            return MeterReading {
                peak_db: f32::NEG_INFINITY,
                rms_db: f32::NEG_INFINITY,
                peak_hold_db: self.peak_hold_db,
                is_clipping: self.is_clipping,
            };
        }

        let mut max_abs: f32 = 0.0;
        let mut sum_sq: f32 = 0.0;

        for &s in samples {
            let abs_s = s.abs();
            if abs_s > max_abs {
                max_abs = abs_s;
            }
            sum_sq += s * s;
        }

        let rms = (sum_sq / samples.len() as f32).sqrt();

        let peak_db = if max_abs > 0.0 {
            20.0 * max_abs.log10()
        } else {
            f32::NEG_INFINITY
        };

        let rms_db = if rms > 0.0 {
            20.0 * rms.log10()
        } else {
            f32::NEG_INFINITY
        };

        if max_abs > 1.0 || peak_db > 0.0 {
            self.is_clipping = true;
        }

        if peak_db > self.peak_hold_db {
            self.peak_hold_db = peak_db;
        }

        MeterReading {
            peak_db,
            rms_db,
            peak_hold_db: self.peak_hold_db,
            is_clipping: self.is_clipping,
        }
    }

    pub fn reset_peak_hold(&mut self) {
        self.peak_hold_db = f32::NEG_INFINITY;
        self.is_clipping = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_meters() {
        let mut meter = AudioMeter::new();

        // 1. Feed known sine wave (-12 dBFS)
        // Amplitude A = 10^(-12 / 20) ≈ 0.25118864
        let target_amplitude = 10f32.powf(-12.0 / 20.0);
        let sample_rate = 48000;
        let freq = 1000.0; // 1 kHz sine
        let num_samples = 48000;
        let mut sine_samples = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let t = i as f32 / sample_rate as f32;
            let val = target_amplitude * (2.0 * std::f32::consts::PI * freq * t).sin();
            sine_samples.push(val);
        }

        let reading = meter.process_samples(&sine_samples);
        assert!(
            (reading.peak_db - (-12.0)).abs() <= 0.5,
            "Expected peak ~ -12 dBFS (±0.5 dB), got {}",
            reading.peak_db
        );
        assert!(!reading.is_clipping);

        // 2. Feed silence
        let silence = vec![0.0f32; 1000];
        let silence_reading = meter.process_samples(&silence);
        assert!(
            silence_reading.peak_db < -90.0 || silence_reading.peak_db.is_infinite(),
            "Expected silence to be <-90 dBFS or -inf, got {}",
            silence_reading.peak_db
        );

        // 3. Feed clipping signal (+1 dBFS)
        // Amplitude A = 10^(+1 / 20) ≈ 1.122018
        let clip_amplitude = 10f32.powf(1.0 / 20.0);
        let mut clip_samples = Vec::with_capacity(1000);
        for i in 0..1000 {
            let t = i as f32 / sample_rate as f32;
            clip_samples.push(clip_amplitude * (2.0 * std::f32::consts::PI * freq * t).sin());
        }

        let clip_reading = meter.process_samples(&clip_samples);
        assert!(clip_reading.is_clipping, "Clipping flag must activate on > 0 dBFS signal");
        assert!(clip_reading.peak_hold_db > 0.0, "Peak hold must register level > 0 dBFS");
        assert!(
            (clip_reading.peak_hold_db - 1.0).abs() <= 0.5,
            "Expected peak hold around +1.0 dBFS, got {}",
            clip_reading.peak_hold_db
        );
    }
}
