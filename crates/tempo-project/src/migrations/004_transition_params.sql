-- Transitions as objects (0.4.0): the colour of a Dip To Color dissolve ('#rrggbb',
-- NULL = black) and Resolve's Ease setting (0=none 1=in 2=out 3=in and out).
-- Kind 9 = Dip To Color. Cross dissolves of older files still live in the clips'
-- properties (dissolve_in_us) and become transitions when the file loads.
ALTER TABLE transitions ADD COLUMN color TEXT;
ALTER TABLE transitions ADD COLUMN ease INTEGER NOT NULL DEFAULT 0;
