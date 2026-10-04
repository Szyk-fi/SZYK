-- musicutil for the Portamax norns cartridge: scales, note names and
-- conversions. Scale interval sets are standard music theory; the order
-- of the first entries follows the order norns scripts index into.
local MusicUtil = {}

MusicUtil.NOTE_NAMES = { "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B" }

MusicUtil.SCALES = {
  { name = "Major", alt_names = { "Ionian" }, intervals = { 0, 2, 4, 5, 7, 9, 11, 12 } },
  { name = "Natural Minor", alt_names = { "Minor", "Aeolian" }, intervals = { 0, 2, 3, 5, 7, 8, 10, 12 } },
  { name = "Harmonic Minor", intervals = { 0, 2, 3, 5, 7, 8, 11, 12 } },
  { name = "Melodic Minor", intervals = { 0, 2, 3, 5, 7, 9, 11, 12 } },
  { name = "Dorian", intervals = { 0, 2, 3, 5, 7, 9, 10, 12 } },
  { name = "Phrygian", intervals = { 0, 1, 3, 5, 7, 8, 10, 12 } },
  { name = "Lydian", intervals = { 0, 2, 4, 6, 7, 9, 11, 12 } },
  { name = "Mixolydian", intervals = { 0, 2, 4, 5, 7, 9, 10, 12 } },
  { name = "Locrian", intervals = { 0, 1, 3, 5, 6, 8, 10, 12 } },
  { name = "Whole Tone", intervals = { 0, 2, 4, 6, 8, 10, 12 } },
  { name = "Major Pentatonic", alt_names = { "Gagaku Ryo Sen Pou" }, intervals = { 0, 2, 4, 7, 9, 12 } },
  { name = "Minor Pentatonic", alt_names = { "Zokugaku Yo Sen Pou" }, intervals = { 0, 3, 5, 7, 10, 12 } },
  { name = "Major Bebop", intervals = { 0, 2, 4, 5, 7, 8, 9, 11, 12 } },
  { name = "Altered Scale", intervals = { 0, 1, 3, 4, 6, 8, 10, 12 } },
  { name = "Dorian Bebop", intervals = { 0, 2, 3, 4, 5, 7, 9, 10, 12 } },
  { name = "Mixolydian Bebop", intervals = { 0, 2, 4, 5, 7, 9, 10, 11, 12 } },
  { name = "Blues Scale", alt_names = { "Blues" }, intervals = { 0, 3, 5, 6, 7, 10, 12 } },
  { name = "Diminished Whole Half", intervals = { 0, 2, 3, 5, 6, 8, 9, 11, 12 } },
  { name = "Diminished Half Whole", intervals = { 0, 1, 3, 4, 6, 7, 9, 10, 12 } },
  { name = "Neapolitan Major", intervals = { 0, 1, 3, 5, 7, 9, 11, 12 } },
  { name = "Hungarian Major", intervals = { 0, 3, 4, 6, 7, 9, 10, 12 } },
  { name = "Harmonic Major", intervals = { 0, 2, 4, 5, 7, 8, 11, 12 } },
  { name = "Hungarian Minor", intervals = { 0, 2, 3, 6, 7, 8, 11, 12 } },
  { name = "Lydian Minor", intervals = { 0, 2, 4, 6, 7, 8, 10, 12 } },
  { name = "Neapolitan Minor", alt_names = { "Byzantine" }, intervals = { 0, 1, 3, 5, 7, 8, 11, 12 } },
  { name = "Major Locrian", intervals = { 0, 2, 4, 5, 6, 8, 10, 12 } },
  { name = "Leading Whole Tone", intervals = { 0, 2, 4, 6, 8, 10, 11, 12 } },
  { name = "Six Tone Symmetrical", intervals = { 0, 1, 4, 5, 8, 9, 11, 12 } },
  { name = "Balinese", intervals = { 0, 1, 3, 7, 8, 12 } },
  { name = "Persian", intervals = { 0, 1, 4, 5, 6, 8, 11, 12 } },
  { name = "East Indian Purvi", intervals = { 0, 1, 4, 6, 7, 8, 11, 12 } },
  { name = "Oriental", intervals = { 0, 1, 4, 5, 6, 9, 10, 12 } },
  { name = "Double Harmonic", intervals = { 0, 1, 4, 5, 7, 8, 11, 12 } },
  { name = "Enigmatic", intervals = { 0, 1, 4, 6, 8, 10, 11, 12 } },
  { name = "Overtone", intervals = { 0, 2, 4, 6, 7, 9, 10, 12 } },
  { name = "Eight Tone Spanish", intervals = { 0, 1, 3, 4, 5, 6, 8, 10, 12 } },
  { name = "Prometheus", intervals = { 0, 2, 4, 6, 9, 10, 12 } },
  { name = "Gagaku Rittsu Sen Pou", intervals = { 0, 2, 5, 7, 9, 10, 12 } },
  { name = "In Sen Pou", intervals = { 0, 1, 5, 7, 10, 12 } },
  { name = "Okinawa", intervals = { 0, 4, 5, 7, 11, 12 } },
  { name = "Chromatic", intervals = { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12 } },
}

local function find_scale(s)
  if type(s) == "number" then return MusicUtil.SCALES[util.clamp(math.floor(s), 1, #MusicUtil.SCALES)] end
  local want = string.lower(tostring(s))
  for _, sc in ipairs(MusicUtil.SCALES) do
    if string.lower(sc.name) == want then return sc end
    for _, a in ipairs(sc.alt_names or {}) do
      if string.lower(a) == want then return sc end
    end
  end
  return MusicUtil.SCALES[1]
end

function MusicUtil.generate_scale_of_length(root, scale, length)
  local sc = find_scale(scale)
  local steps = #sc.intervals - 1 -- last interval is the octave
  local out = {}
  for i = 0, length - 1 do
    local oct = i // steps
    local deg = i % steps
    local n = root + oct * 12 + sc.intervals[deg + 1]
    if n > 127 then break end
    out[#out + 1] = n
  end
  return out
end

function MusicUtil.generate_scale(root, scale, octaves)
  local sc = find_scale(scale)
  return MusicUtil.generate_scale_of_length(root, scale, (#sc.intervals - 1) * (octaves or 1) + 1)
end

function MusicUtil.note_num_to_name(n, with_octave)
  local name = MusicUtil.NOTE_NAMES[(math.floor(n) % 12) + 1]
  if with_octave then name = name .. (math.floor(n / 12) - 1) end
  return name
end

function MusicUtil.note_nums_to_names(t, with_octave)
  local o = {}
  for i, n in ipairs(t) do o[i] = MusicUtil.note_num_to_name(n, with_octave) end
  return o
end

function MusicUtil.note_num_to_freq(n) return 440 * 2 ^ ((n - 69) / 12) end
function MusicUtil.note_nums_to_freqs(t)
  local o = {}
  for i, n in ipairs(t) do o[i] = MusicUtil.note_num_to_freq(n) end
  return o
end
function MusicUtil.freq_to_note_num(f) return util.clamp(math.floor(12 * math.log(f / 440, 2) + 69.5), 0, 127) end
function MusicUtil.interval_to_ratio(i) return 2 ^ (i / 12) end
function MusicUtil.ratio_to_interval(r) return 12 * math.log(r, 2) end

function MusicUtil.snap_note_to_array(n, arr)
  local best, dist = arr[1], math.huge
  for _, v in ipairs(arr) do
    local d = math.abs(v - n)
    if d < dist then best, dist = v, d end
  end
  return best
end

function MusicUtil.snap_notes_to_array(t, arr)
  local o = {}
  for i, n in ipairs(t) do o[i] = MusicUtil.snap_note_to_array(n, arr) end
  return o
end

MusicUtil.CHORDS = {
  { name = "Major", intervals = { 0, 4, 7 } },
  { name = "Minor", intervals = { 0, 3, 7 } },
  { name = "Major 7", intervals = { 0, 4, 7, 11 } },
  { name = "Minor 7", intervals = { 0, 3, 7, 10 } },
  { name = "Dominant 7", intervals = { 0, 4, 7, 10 } },
  { name = "Sus2", intervals = { 0, 2, 7 } },
  { name = "Sus4", intervals = { 0, 5, 7 } },
  { name = "Diminished", intervals = { 0, 3, 6 } },
  { name = "Augmented", intervals = { 0, 4, 8 } },
}

function MusicUtil.generate_chord(root, name, inversion)
  local want = string.lower(name or "major")
  for _, c in ipairs(MusicUtil.CHORDS) do
    if string.lower(c.name) == want then
      local o = {}
      for i, iv in ipairs(c.intervals) do o[i] = root + iv end
      for _ = 1, (inversion or 0) do
        local n = table.remove(o, 1)
        o[#o + 1] = n + 12
      end
      return o
    end
  end
  return nil
end

return MusicUtil
