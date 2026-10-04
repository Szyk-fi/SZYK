-- breath
-- a Portamax norns script
--
-- a breathing pacer. follow the
-- circle: breathe in as it grows,
-- hold, breathe out as it shrinks.
-- a soft pad climbs with the in-breath,
-- hovers on the hold and falls with
-- the out-breath; each breath a new chord.
--
-- E2 pattern   E3 pace
-- K2 start over   K3 pause
-- pads: a chime
-- (params: root, pad level)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local PATTERNS = {
  { name = "box 4-4-4-4", t = { 4, 4, 4, 4 } },
  { name = "calm 4-7-8", t = { 4, 7, 8, 0 } },
  { name = "even 5-5", t = { 5, 0, 5, 0 } },
  { name = "long out 4-2-6", t = { 4, 2, 6, 0 } },
}
local PHASES = { "breathe in", "hold", "breathe out", "rest" }
local PROG = { { 0, "major 7" }, { 9, "minor 7" }, { 5, "major 7" }, { 7, "sus4" } }
local phase, t = 1, 0
local breaths = 0
local size = 0
local paused = false
local chord = {}
local step_i = 0

local function build()
  local p = PROG[breaths % #PROG + 1]
  local c = MusicUtil.generate_chord(params:get("root") + p[1], p[2], 0)
  chord = {}
  for o = 0, 1 do for _, n in ipairs(c) do chord[#chord + 1] = n + 12 * o end end
end

local function pad(n, amp)
  engine.release(3.5) engine.amp(amp * params:get("level")) engine.cutoff(1100)
  engine.pw(0.5) engine.pan((math.random() - 0.5) * 0.6)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function length(p) return PATTERNS[params:get("pattern")].t[p] * params:get("pace") end

function init()
  local names = {}
  for i, p in ipairs(PATTERNS) do names[i] = p.name end
  params:add_separator("BREATH")
  params:add_option("pattern", "pattern", names, 1)
  params:add_control("pace", "pace", controlspec.new(0.6, 1.6, 'lin', 0.05, 1, 'x'))
  params:add_number("root", "root", 41, 60, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build)
  params:add_control("level", "pad level", controlspec.new(0.2, 1.5, 'lin', 0, 1, ''))
  params:default()
  build()
  pad(chord[1] - 12, 0.3)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      engine.release(4) engine.amp(0.15) engine.cutoff(3000) engine.pw(0.5)
      engine.hz(MusicUtil.note_num_to_freq(chord[(msg.note - 60) % #chord + 1] + 24))
    end
  end
  -- the pad moves in slow steps
  clock.run(function()
    while true do
      clock.sleep(0.45 * params:get("pace"))
      if not paused then
        step_i = step_i + 1
        if phase == 1 then
          local k = util.clamp(math.floor(t / length(1) * #chord) + 1, 1, #chord)
          pad(chord[k], 0.22)
        elseif phase == 3 then
          local k = util.clamp(#chord - math.floor(t / length(3) * #chord), 1, #chord)
          pad(chord[k], 0.18)
        elseif step_i % 3 == 0 then
          pad(phase == 2 and chord[#chord] or chord[1] - 12, 0.14)
        end
      end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      if not paused then advance(1 / 30) end
      redraw()
    end
  end)
end

function advance(dt)
  t = t + dt
  while t >= length(phase) do
    t = t - length(phase)
    repeat -- skip phases this pattern leaves out
      phase = phase % 4 + 1
      if phase == 1 then breaths = breaths + 1 build() pad(chord[1] - 12, 0.3) end
    until length(phase) > 0
  end
  local f = length(phase) > 0 and t / length(phase) or 1
  if phase == 1 then size = (1 - math.cos(f * math.pi)) / 2
  elseif phase == 2 then size = 1
  elseif phase == 3 then size = (1 + math.cos(f * math.pi)) / 2
  else size = 0 end
end

function enc(n, d)
  if n == 2 then params:delta("pattern", d) phase, t = 1, 0
  elseif n == 3 then params:delta("pace", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then phase, t, breaths = 1, 0, 0 build() pad(chord[1] - 12, 0.3)
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  local r = 4 + size * 18
  screen.level(2)
  screen.circle(64, 34, 22) screen.stroke()
  screen.level(phase == 2 and 12 or 7)
  screen.circle(64, 34, r) screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "breath (paused)" or "breath")
  screen.move(127, 8)
  screen.text_right(PHASES[phase])
  local left = math.max(0, length(phase) - t)
  screen.level(phase == 2 and 0 or 15)
  screen.move(64, 37)
  screen.text_center(tostring(math.ceil(left)))
  screen.level(4)
  screen.move(0, 62)
  screen.text(PATTERNS[params:get("pattern")].name)
  screen.move(127, 62)
  screen.text_right(breaths .. " breaths")
  screen.update()
end
