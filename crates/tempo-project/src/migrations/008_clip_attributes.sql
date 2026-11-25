-- Clip Attributes (Tier 1): display name, video frame rate conform, and audio channel mapping.
ALTER TABLE media_sources ADD COLUMN display_name TEXT;
ALTER TABLE media_sources ADD COLUMN conform_fps_num INTEGER;
ALTER TABLE media_sources ADD COLUMN conform_fps_den INTEGER;
ALTER TABLE media_sources ADD COLUMN audio_channel_map TEXT NOT NULL DEFAULT 'stereo';

