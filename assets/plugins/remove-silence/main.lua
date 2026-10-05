-- Remove Silence: cuts stretches quieter than THRESHOLD_DB that last at least
-- MIN_GAP seconds out of the selected audio clips, and closes the gaps.

local THRESHOLD_DB = -40
local MIN_GAP = 0.6
local WINDOW = 0.05

function remove_silence()
    local removed = 0
    for _, id in ipairs(tempo.selection()) do
        local clip = tempo.timeline.clip(id)
        if clip and clip.kind == "audio" then
            local levels = tempo.media.loudness(id, WINDOW)
            -- Collect the quiet stretches as {from, to} in timeline seconds.
            local gaps, quiet_from = {}, nil
            for i = 1, #levels + 1 do
                local t = clip.start + (i - 1) * WINDOW
                local quiet = i <= #levels and levels[i] < THRESHOLD_DB
                if quiet and not quiet_from then
                    quiet_from = t
                elseif not quiet and quiet_from then
                    if t - quiet_from >= MIN_GAP then
                        gaps[#gaps + 1] = { quiet_from, t }
                    end
                    quiet_from = nil
                end
            end
            -- Work from the end backwards so earlier times stay valid.
            for i = #gaps, 1, -1 do
                local from, to = gaps[i][1], gaps[i][2]
                local piece = id
                if to < clip.start + clip.duration - 0.001 then
                    tempo.timeline.split(id, to)
                end
                if from > clip.start + 0.001 then
                    piece = tempo.timeline.split(id, from)
                end
                tempo.timeline.delete(piece, { ripple = true })
                removed = removed + 1
                if piece == id then
                    break -- the whole start of the clip went; `id` no longer exists
                end
            end
        end
    end
    if removed == 0 then
        tempo.notify("No silence found. Select an audio clip first.")
    else
        tempo.notify("Removed " .. removed .. " silent part" .. (removed == 1 and "" or "s"))
    end
end
