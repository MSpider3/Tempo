//! Chapters made from named timeline markers.

use tempo_timeline::Marker;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    /// Start, relative to the beginning of the exported range.
    pub start_us: i64,
    pub title: String,
}

/// Named markers inside `[range_start, range_end)` become chapters. Unnamed
/// markers are editing notes and are left out.
pub fn chapters_from_markers(markers: &[Marker], range_start: i64, range_end: i64) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = markers
        .iter()
        .filter(|m| !m.name.trim().is_empty() && m.position_us >= range_start && m.position_us < range_end)
        .map(|m| Chapter { start_us: m.position_us - range_start, title: m.name.trim().to_string() })
        .collect();
    chapters.sort_by_key(|c| c.start_us);
    chapters
}

/// Reasons YouTube would not turn this list into chapters. Empty means it is fine.
pub fn youtube_problems(chapters: &[Chapter], total_us: i64) -> Vec<String> {
    let mut problems = Vec::new();
    if chapters.len() < 3 {
        problems.push(format!("YouTube needs at least 3 chapters; there are {}.", chapters.len()));
    }
    if chapters.first().is_some_and(|c| c.start_us >= 1_000_000) {
        problems.push("The first chapter must start at 00:00.".to_string());
    }
    let ends = chapters.iter().skip(1).map(|c| c.start_us).chain(std::iter::once(total_us));
    for (chapter, end) in chapters.iter().zip(ends) {
        if end - chapter.start_us < 10_000_000 {
            problems.push(format!("\"{}\" is shorter than 10 seconds.", chapter.title));
        }
    }
    problems
}

/// `00:00 Intro` lines, the form video sites read from a description.
pub fn format_chapter_list(chapters: &[Chapter]) -> String {
    let long = chapters.last().is_some_and(|c| c.start_us >= 3_600_000_000);
    chapters
        .iter()
        .map(|c| {
            let s = c.start_us / 1_000_000;
            if long {
                format!("{}:{:02}:{:02} {}", s / 3600, (s / 60) % 60, s % 60, c.title)
            } else {
                format!("{:02}:{:02} {}", s / 60, s % 60, c.title)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// FFmpeg metadata file content that embeds the chapters in the output file.
pub fn ffmetadata(chapters: &[Chapter], total_us: i64) -> String {
    let mut out = String::from(";FFMETADATA1\n");
    let ends = chapters.iter().skip(1).map(|c| c.start_us).chain(std::iter::once(total_us));
    for (chapter, end) in chapters.iter().zip(ends) {
        // FFmpeg metadata files need these characters escaped with a backslash.
        let mut title = String::new();
        for ch in chapter.title.chars() {
            match ch {
                '\\' | '=' | ';' | '#' => {
                    title.push('\\');
                    title.push(ch);
                }
                '\n' => title.push(' '),
                _ => title.push(ch),
            }
        }
        out.push_str(&format!(
            "[CHAPTER]\nTIMEBASE=1/1000000\nSTART={}\nEND={}\ntitle={}\n",
            chapter.start_us,
            end.max(chapter.start_us + 1),
            title
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::MarkerColor;

    fn marker(sec: i64, name: &str) -> Marker {
        Marker::new(sec * 1_000_000, name, MarkerColor::Blue)
    }

    #[test]
    fn only_named_markers_in_range_become_chapters() {
        let markers = vec![marker(0, "Intro"), marker(20, ""), marker(30, " Setup "), marker(90, "Late")];
        let ch = chapters_from_markers(&markers, 0, 60_000_000);
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[1], Chapter { start_us: 30_000_000, title: "Setup".into() });
    }

    #[test]
    fn times_are_relative_to_the_range() {
        let ch = chapters_from_markers(&[marker(40, "A")], 30_000_000, 100_000_000);
        assert_eq!(ch[0].start_us, 10_000_000);
    }

    #[test]
    fn youtube_rules() {
        let good = chapters_from_markers(&[marker(0, "A"), marker(15, "B"), marker(40, "C")], 0, 60_000_000);
        assert!(youtube_problems(&good, 60_000_000).is_empty());

        let late = chapters_from_markers(&[marker(5, "A"), marker(20, "B"), marker(40, "C")], 0, 60_000_000);
        assert!(youtube_problems(&late, 60_000_000).iter().any(|p| p.contains("00:00")));

        let few = chapters_from_markers(&[marker(0, "A"), marker(20, "B")], 0, 60_000_000);
        assert!(youtube_problems(&few, 60_000_000).iter().any(|p| p.contains("at least 3")));

        let short = chapters_from_markers(&[marker(0, "A"), marker(4, "B"), marker(40, "C")], 0, 60_000_000);
        assert!(youtube_problems(&short, 60_000_000).iter().any(|p| p.contains("\"A\"")));
    }

    #[test]
    fn list_format() {
        let ch = chapters_from_markers(&[marker(0, "Intro"), marker(85, "Setup")], 0, 200_000_000);
        assert_eq!(format_chapter_list(&ch), "00:00 Intro\n01:25 Setup");
    }
}
