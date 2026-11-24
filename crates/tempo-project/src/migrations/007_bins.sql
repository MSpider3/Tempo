-- Media Pool bins for timelines (0.5.0). NULL or Master UUID means root Master bin.
ALTER TABLE timelines ADD COLUMN bin_id TEXT;
