-- loopdrift
-- a Portamax norns script
--
-- five loops of unequal,
-- unrelated lengths each hold a
-- few long notes. they never
-- line up the same way twice,
-- so the music keeps finding
-- new chords by itself.
--
-- E2 time scale   E3 warmth
-- K2 new notes    K3 bring loops home
-- pads: add a note to a loop
-- (params: key, scale, haze)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

-- lengths in seconds; no two share a simple ratio
local LENGTHS = { 17.3, 21.1, 25.7, 29.9, 33.4 }
local loops = {}
local scale = {}
local elapsed = 0
local last_note_name = ""
local SCALES = { "Major", "Dorian", "Lydian", "Major Pentatonic", "Natural Minor" }

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("key") - 12, SCALES[params:get("scale")], 22)
end

local function seed_loop(i)
  local L = loops[i]
  L.notes = {}
  local n = math.random(1, 3)
  for k = 1, n do
    -- each loop lives in its own register: loop 1 low, loop 5 high
    local deg = util.clamp((i - 1) * 3 + math.random(1, 7), 1, 22)
    table.insert(L.notes, { at = (k == 1 and i == 1) and 0 or math.random(), deg = deg, glow = 0 })
  end
end

local function new_notes()
  for i = 1, #LENGTHS do seed_loop(i) end
end

local function sound(i, nt)
  local note = scale[nt.deg]
  engine.pan(util.linlin(1, #LENGTHS, -0.7, 0.7, i))
  engine.pw(0.3 + i * 0.08)
  engine.cutoff(params:get("warmth") * (0.7 + i * 0.15))
  engine.release(params:get("sustain") * (1.2 - i * 0.08))
  engine.amp(0.2 - i * 0.015)
  engine.hz(MusicUtil.note_num_to_freq(note))
  -- a quiet octave above gives each long note a little shimmer
  engine.amp(0.04)
  engine.hz(MusicUtil.note_num_to_freq(note + 12) * 1.003)
  nt.glow = 15
  last_note_name = MusicUtil.note_num_to_name(note, true)
end

local function setup_haze()
  audio.level_eng_cut(0.8)
  softcut.buffer_clear()
  softcut.enable(1, 1)
  softcut.buffer(1, 1)
  softcut.level(1, 0.55)
  softcut.pan(1, 0)
  softcut.rate(1, 1)
  softcut.loop(1, 1)
  softcut.loop_start(1, 0)
  softcut.loop_end(1, 3.7)
  softcut.position(1, 0)
  softcut.fade_time(1, 0.2)
  softcut.level_input_cut(1, 1, 0.8)
  softcut.level_input_cut(2, 1, 0.8)
  softcut.rec_level(1, 1)
  softcut.pre_level(1, params:get("haze"))
  softcut.play(1, 1)
  softcut.rec(1, 1)
  softcut.filter_dry(1, 0)
  softcut.filter_lp(1, 1)
  softcut.filter_fc(1, 1600)
  softcut.filter_rq(1, 2)
end

local function advance(dt)
  dt = dt * params:get("rate")
  elapsed = elapsed + dt
  for i, L in ipairs(loops) do
    local before = L.phase
    L.phase = L.phase + dt / L.len
    local wrapped = L.phase >= 1
    if wrapped then L.phase = L.phase - 1 end
    for _, nt in ipairs(L.notes) do
      local crossed
      if wrapped then
        crossed = nt.at >= before or nt.at < L.phase
      else
        crossed = nt.at >= before and nt.at < L.phase
      end
      if crossed then sound(i, nt) end
      nt.glow = math.max(0, nt.glow - dt * 6)
    end
  end
end

local function home()
  -- every loop back to its start; the downbeat of everything at once
  for _, L in ipairs(loops) do L.phase = -0.00001 end
  elapsed = 0
end

function init()
  local names = {}
  for i, s in ipairs(SCALES) do names[i] = string.lower(s) end
  params:add_separator("LOOPDRIFT")
  params:add_number("key", "key", 48, 64, 55, function(p) return MusicUtil.note_num_to_name(p:get(), false) end)
  params:set_action("key", build_scale)
  params:add_option("scale", "scale", names, 4)
  params:set_action("scale", build_scale)
  params:add_control("rate", "time scale", controlspec.new(0.25, 4, 'exp', 0, 1, 'x'))
  params:add_control("warmth", "warmth", controlspec.new(200, 4000, 'exp', 0, 1100, 'hz'))
  params:add_control("sustain", "sustain", controlspec.new(1, 12, 'lin', 0, 5, 's'))
  params:add_control("haze", "haze", controlspec.new(0, 0.9, 'lin', 0, 0.7, ''))
  params:set_action("haze", function(x) softcut.pre_level(1, x) end)
  params:default()
  engine.gain(0.5)
  math.randomseed(os.time())
  build_scale()
  for i, len in ipairs(LENGTHS) do loops[i] = { len = len, phase = 0, notes = {} } end
  new_notes()
  home()
  setup_haze()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      -- the pad's note goes into a loop, at the loop's current place
      local i = (msg.note % #LENGTHS) + 1
      local L = loops[i]
      local snapped = MusicUtil.snap_note_to_array(msg.note, scale)
      local deg = 1
      for k, n in ipairs(scale) do if n == snapped then deg = k end end
      local nt = { at = L.phase, deg = deg, glow = 0 }
      table.insert(L.notes, nt)
      if #L.notes > 5 then table.remove(L.notes, 1) end
      sound(i, nt)
    end
  end
  local last = util.time()
  local frame = metro.init(function()
    local now = util.time()
    advance(math.min(0.1, now - last))
    last = now
    redraw()
  end, 1 / 30)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("rate", d)
  elseif n == 3 then params:delta("warmth", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_notes()
  elseif n == 3 then home() end
  redraw()
end

function redraw()
  screen.clear()
  local X0, X1 = 22, 124
  for i, L in ipairs(loops) do
    local y = 4 + i * 10
    -- the loop's length as a label
    screen.level(4)
    screen.move(0, y + 2)
    screen.text(string.format("%.1f", L.len))
    -- its track
    screen.level(2)
    screen.move(X0, y)
    screen.line(X1, y)
    screen.stroke()
    -- its notes, glowing when struck
    for _, nt in ipairs(L.notes) do
      local x = X0 + nt.at * (X1 - X0)
      screen.level(math.max(5, math.floor(nt.glow)))
      screen.rect(x - 1, y - 3, 3, 6)
      screen.fill()
    end
    -- its playhead
    local px = X0 + math.max(0, L.phase) * (X1 - X0)
    screen.level(15)
    screen.move(px, y - 4)
    screen.line(px, y + 4)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 6)
  screen.text("loopdrift")
  screen.level(5)
  screen.move(127, 6)
  screen.text_right(last_note_name)
  screen.level(3)
  screen.move(127, 63)
  screen.text_right(util.s_to_hms(elapsed))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
