-- Resolve parity (0.5.0). Timelines: their own frame rate when "Use Project Settings" was
-- off (NULL = the project's). Transitions: the Inspector's Video switch (0 = plays as a cut).
-- Marker colours 6-16 are Resolve's other colours; the column was already an integer.
ALTER TABLE timelines ADD COLUMN fps_num INTEGER;
ALTER TABLE timelines ADD COLUMN fps_den INTEGER;
ALTER TABLE transitions ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
