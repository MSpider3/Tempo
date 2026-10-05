use tempo_timeline::types::{us_to_frame, Clip, RationalFps, Timeline, TrackKind};

pub fn us_to_timecode(us: i64, fps: RationalFps) -> String {
    let fps_f = fps.to_f64();
    let fps_int = (fps_f.round() as i64).max(1);
    let frames = us_to_frame(us, fps).max(0);
    let ff = frames % fps_int;
    let total_secs = frames / fps_int;
    let ss = total_secs % 60;
    let mm = (total_secs / 60) % 60;
    let hh = total_secs / 3600;
    format!("{:02}:{:02}:{:02}:{:02}", hh, mm, ss, ff)
}

pub fn export_timeline_to_edl(timeline: &Timeline, title: &str, fps: RationalFps) -> String {
    let mut out = String::new();
    let title_clean = if title.trim().is_empty() {
        "UNTITLED_PROJECT"
    } else {
        title.trim()
    };

    out.push_str(&format!("TITLE: {}\n", title_clean));
    out.push_str("FCM: NON-DROP FRAME\n\n");

    // Collect all video clips across video tracks
    let mut clips: Vec<&Clip> = Vec::new();
    for track in &timeline.tracks {
        if track.kind == TrackKind::Video {
            for clip in &track.clips {
                clips.push(clip);
            }
        }
    }

    // Sort clips by timeline_in
    clips.sort_by_key(|c| c.timeline_in);

    for (idx, clip) in clips.iter().enumerate() {
        let event_num = idx + 1;
        let reel = "AX";
        let track_type = "V";
        let cut_type = "C";

        let src_in_tc = us_to_timecode(clip.source_in, fps);
        let src_out_tc = us_to_timecode(clip.source_out, fps);
        let rec_in_tc = us_to_timecode(clip.timeline_in, fps);
        let rec_out_tc = us_to_timecode(clip.timeline_out, fps);

        out.push_str(&format!(
            "{:03}  {:<8} {:<5} {:<8} {} {} {} {}\n",
            event_num, reel, track_type, cut_type, src_in_tc, src_out_tc, rec_in_tc, rec_out_tc
        ));
        out.push_str(&format!("* FROM CLIP NAME: {}\n\n", clip.name));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::types::{Clip, ClipType, RationalFps, Timeline, Track};
    use uuid::Uuid;

    #[test]
    fn test_edl_export() {
        let mut timeline = Timeline::new();
        let track = Track::new(TrackKind::Video, 1, "V1", 0);
        let track_id = track.id;
        timeline.tracks.push(track);

        // Add 3 clips
        let clip1 = Clip::new(
            track_id,
            Uuid::new_v4(),
            ClipType::Video,
            "Intro.mp4",
            0,
            2_000_000,
            0,
            2_000_000,
        );
        let clip2 = Clip::new(
            track_id,
            Uuid::new_v4(),
            ClipType::Video,
            "MainScene.mp4",
            2_000_000,
            7_000_000,
            1_000_000,
            6_000_000,
        );
        let clip3 = Clip::new(
            track_id,
            Uuid::new_v4(),
            ClipType::Video,
            "Outro.mp4",
            7_000_000,
            10_000_000,
            0,
            3_000_000,
        );

        timeline.tracks[0].clips.push(clip1);
        timeline.tracks[0].clips.push(clip2);
        timeline.tracks[0].clips.push(clip3);

        let edl_output = export_timeline_to_edl(&timeline, "MY_AWESOME_FILM", RationalFps::FPS_24);

        // Assert valid EDL format
        assert!(edl_output.contains("TITLE: MY_AWESOME_FILM"));
        assert!(edl_output.contains("FCM: NON-DROP FRAME"));

        // Assert 3 events present
        assert!(edl_output.contains("001  AX       V     C"));
        assert!(edl_output.contains("002  AX       V     C"));
        assert!(edl_output.contains("003  AX       V     C"));

        // Assert clip names
        assert!(edl_output.contains("* FROM CLIP NAME: Intro.mp4"));
        assert!(edl_output.contains("* FROM CLIP NAME: MainScene.mp4"));
        assert!(edl_output.contains("* FROM CLIP NAME: Outro.mp4"));

        // Assert valid timecodes
        assert!(edl_output.contains("00:00:00:00 00:00:02:00 00:00:00:00 00:00:02:00"));
        assert!(edl_output.contains("00:00:01:00 00:00:06:00 00:00:02:00 00:00:07:00"));
    }
}
