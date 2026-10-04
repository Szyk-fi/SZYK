-- heartbeat
-- a Portamax norns script
--
-- a resting heart: lub-dub, lub-dub.
-- the gap between beats is never
-- quite the same - it speeds up as
-- the breath comes in and slows as
-- it goes out, with a little drift.
-- a quiet melody floats above it.
--
-- E2 heart rate   E3 variability
-- K2 startle   K3 rest / resume
-- pads: a passing thought (note)
-- (params: root, melody)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local ecg = {}
local beat_t = 0 -- time since the last lub, for drawing
local rr = 1
local breath = 0
local startle = 0
local resting = false
local pulse = 0
local scale = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root") + 24, "Major Pentatonic", 8)
end

local function thump(note, amp, cut)
  engine.release(0.35) engine.amp(amp) engine.cutoff(cut) engine.pw(0.15) engine.pan(0)
  engine.hz(MusicUtil.note_num_to_freq(note))
end

local function float_note(n)
  engine.release(2.5) engine.amp(0.12) engine.cutoff(1800) engine.pw(0.5)
  engine.pan(math.random() - 0.5)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

-- a rough PQRST shape over one beat, t in seconds since the lub
local function wave(t)
  if t < 0.08 then return math.sin(t / 0.08 * math.pi) * 2 end
  if t < 0.1 then return 0 end
  if t < 0.13 then return -3 end
  if t < 0.2 then return 16 end
  if t < 0.24 then return -5 end
  if t > 0.32 and t < 0.48 then return math.sin((t - 0.32) / 0.16 * math.pi) * 4 end
  return 0
end

function init()
  params:add_separator("HEARTBEAT")
  params:add_number("rate", "heart rate", 40, 140, 64, function(p) return p:get() .. " bpm" end)
  params:add_control("hrv", "variability", controlspec.new(0, 0.2, 'lin', 0, 0.06, ''))
  params:add_number("root", "root", 33, 48, 38, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("melody", "melody", controlspec.new(0, 1, 'lin', 0, 0.4, ''))
  params:default()
  math.randomseed(os.time())
  build_scale()
  for i = 1, 128 do ecg[i] = 0 end
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then float_note(scale[(msg.note - 60) % #scale + 1]) end
  end
  clock.run(function()
    while true do
      if resting then clock.sleep(0.1) else
        -- breathing (about 4 s a cycle) speeds the heart on the in-breath
        local bpm = params:get("rate") * (1 + startle)
        local rsa = math.sin(breath * 2 * math.pi) * params:get("hrv")
        local jitter = (math.random() - 0.5) * params:get("hrv") * 0.5
        rr = 60 / bpm * (1 - rsa + jitter)
        beat_t, pulse = 0, 8
        thump(params:get("root"), 0.35, 500)
        local gap = 0.12 + 0.18 * math.sqrt(rr)
        clock.sleep(gap)
        thump(params:get("root") + 7, 0.22, 700)
        if math.random() < params:get("melody") then float_note(scale[math.random(#scale)]) end
        clock.sleep(math.max(0.1, rr - gap))
        startle = startle * 0.85
      end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      breath = (breath + 1 / 30 / 4) % 1
      beat_t = beat_t + 1 / 30
      table.remove(ecg, 1)
      table.insert(ecg, resting and 0 or wave(beat_t))
      pulse = math.max(0, pulse - 1)
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("rate", d)
  elseif n == 3 then params:delta("hrv", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then startle = 0.5 thump(params:get("root") + 12, 0.3, 900)
  elseif n == 3 then resting = not resting end
  redraw()
end

function redraw()
  screen.clear()
  screen.level(2)
  screen.move(0, 36) screen.line(100, 36) screen.stroke()
  screen.level(12)
  screen.move(0, 36 - ecg[29])
  for i = 30, 128 do screen.line(i - 29, 36 - ecg[i]) end
  screen.stroke()
  -- the heart itself swells on each beat
  local r = 5 + pulse * 0.6
  screen.level(pulse > 0 and 15 or 6)
  screen.circle(110 - r * 0.45, 30, r * 0.55) screen.fill()
  screen.circle(110 + r * 0.45, 30, r * 0.55) screen.fill()
  screen.move(110 - r, 31) screen.line(110, 31 + r * 1.2) screen.line(110 + r, 31) screen.close() screen.fill()
  -- breath gauge
  screen.level(3)
  screen.rect(104, 46, 12, 2) screen.fill()
  screen.level(8)
  screen.rect(104 + (math.sin(breath * 2 * math.pi) + 1) * 5, 45, 2, 4) screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(resting and "heartbeat (rest)" or "heartbeat")
  screen.level(4)
  screen.move(0, 62)
  screen.text(math.floor(60 / rr + 0.5) .. " bpm now")
  screen.move(127, 62)
  screen.text_right("hrv " .. params:string("hrv"))
  screen.update()
end
