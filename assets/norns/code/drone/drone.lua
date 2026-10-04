-- drone
-- a Portamax norns script
--
-- a slow chord drone. long notes
-- are layered into a softcut loop
-- that never quite repeats, while
-- the chord drifts from one
-- neighbour to the next.
--
-- E2 drift   E3 haze
-- K2 next chord   K3 hold chord
-- (params: root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local CHORDS = { "minor 7", "major 7", "sus2", "minor", "sus4", "major" }
local chord = {}
local chord_i = 1
local held = false
local glow = {}
local t = 0

local function pick_chord()
  local root = params:get("root") + ({ 0, 5, 7, 3, 10, 8 })[chord_i]
  chord = MusicUtil.generate_chord(root, CHORDS[chord_i], chord_i % 3)
end

local function setup_loop()
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  for v = 1, 2 do
    softcut.enable(v, 1)
    softcut.buffer(v, v)
    softcut.level(v, 0.7)
    softcut.pan(v, v == 1 and -0.7 or 0.7)
    softcut.rate(v, v == 1 and 1 or 0.5)
    softcut.loop(v, 1)
    softcut.loop_start(v, 1)
    softcut.loop_end(v, v == 1 and 7.3 or 11.9)
    softcut.position(v, 1)
    softcut.fade_time(v, 0.3)
    softcut.level_input_cut(1, v, 1.0)
    softcut.level_input_cut(2, v, 1.0)
    softcut.rec_level(v, 0.6)
    softcut.pre_level(v, 0.85)
    softcut.play(v, 1)
    softcut.rec(v, 1)
    softcut.filter_dry(v, 0)
    softcut.filter_lp(v, 1)
    softcut.filter_fc(v, 1800)
    softcut.filter_rq(v, 2)
  end
end

function init()
  params:add_separator("DRONE")
  params:add_number("root", "root", 36, 60, 43, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", pick_chord)
  params:add_control("drift", "drift", controlspec.new(4, 60, 'exp', 0, 16, 'beats'))
  params:add_control("haze", "haze", controlspec.new(0, 0.97, 'lin', 0, 0.85, ''))
  params:set_action("haze", function(x) softcut.pre_level(1, x) softcut.pre_level(2, x) end)
  params:default()
  engine.release(5)
  engine.cutoff(900)
  engine.amp(0.12)
  engine.pw(0.4)
  setup_loop()
  pick_chord()
  clock.run(function()
    local beats = 0
    while true do
      clock.sync(1)
      beats = beats + 1
      if not held and beats >= params:get("drift") then
        beats = 0
        chord_i = chord_i % #CHORDS + 1
        pick_chord()
      end
      if math.random() < 0.6 then
        local i = math.random(#chord)
        engine.pan(math.random() * 1.4 - 0.7)
        engine.hz(MusicUtil.note_num_to_freq(chord[i] + (math.random() < 0.3 and 12 or 0)))
        glow[i] = 15
      end
    end
  end)
  local m = metro.init(function()
    t = t + 1
    for i = 1, 4 do glow[i] = math.max(0, (glow[i] or 0) - 0.3) end
    redraw()
  end, 1 / 20)
  m:start()
end

function enc(n, d)
  if n == 2 then params:delta("drift", d)
  elseif n == 3 then params:delta("haze", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then chord_i = chord_i % #CHORDS + 1 pick_chord()
  elseif n == 3 then held = not held end
end

function redraw()
  screen.clear()
  for i, note in ipairs(chord) do
    local y = 58 - (note - params:get("root")) * 1.6
    screen.level(math.floor(2 + (glow[i] or 0) * 0.85))
    for x = 0, 127, 3 do
      screen.pixel(x, y + math.sin(x / 9 + t / 20 + i) * 2)
      screen.fill()
    end
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("drone: " .. CHORDS[chord_i] .. (held and " (held)" or ""))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
  softcut.rec(2, 0)
end
