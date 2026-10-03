-- chance
-- a Portamax norns script
--
-- a sixteen-step sequencer where
-- every step carries its own odds.
-- tall bars almost always play,
-- short ones only now and then, so
-- the line keeps its shape but
-- never quite repeats.
--
-- E2 select step   E3 step chance
-- K2 new notes   K3 reroll chances
-- pads: set the selected step's note
-- (params: scale, root, bias)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local SCALES = { "Minor Pentatonic", "Dorian", "Major", "Phrygian" }
local steps = {}
local pos = 0
local sel = 1
local fired = {}
local scale = {}

local function build()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), SCALES[params:get("scale")], 12)
  for _, s in ipairs(steps) do s.note = MusicUtil.snap_note_to_array(s.note, scale) end
end

local function new_notes()
  for i = 1, 16 do
    steps[i] = steps[i] or { p = 0.5 }
    steps[i].note = scale[math.random(1, #scale)]
  end
end

local function reroll()
  for i = 1, 16 do
    -- downbeats favoured, offbeats left to fate
    local base = (i % 4 == 1) and 0.85 or ((i % 2 == 1) and 0.5 or 0.25)
    steps[i].p = util.clamp(base + (math.random() - 0.5) * 0.4, 0.05, 1)
  end
end

function init()
  params:add_separator("CHANCE")
  params:add_option("scale", "scale", SCALES, 1)
  params:set_action("scale", build)
  params:add_number("root", "root", 45, 64, 57, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build)
  params:add_control("bias", "bias", controlspec.new(-0.5, 0.5, 'lin', 0, 0, ''))
  params:add_control("tone", "tone", controlspec.new(400, 5000, 'exp', 0, 1800, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:default()
  engine.release(0.6)
  engine.pw(0.35)
  math.randomseed(os.time())
  build()
  new_notes()
  reroll()
  for i = 1, 16 do fired[i] = 0 end
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      steps[sel].note = msg.note - 60 + params:get("root")
      steps[sel].p = math.max(steps[sel].p, 0.6)
    end
  end
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      pos = pos % 16 + 1
      local s = steps[pos]
      if math.random() < util.clamp(s.p + params:get("bias"), 0, 1) then
        engine.amp(0.15 + 0.15 * s.p)
        engine.hz(MusicUtil.note_num_to_freq(s.note))
        fired[pos] = 15
      end
      for i = 1, 16 do fired[i] = math.max(0, fired[i] - 2) end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then sel = util.clamp(sel + d, 1, 16)
  elseif n == 3 then steps[sel].p = util.clamp(steps[sel].p + d * 0.05, 0, 1) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_notes()
  elseif n == 3 then reroll() end
  redraw()
end

function redraw()
  screen.clear()
  for i = 1, 16 do
    local x = 1 + (i - 1) * 8
    local h = math.floor(steps[i].p * 36)
    screen.level(math.max(i == sel and 8 or 2, fired[i]))
    screen.rect(x, 50 - h, 6, h)
    screen.fill()
    screen.level(i == pos and 15 or 1)
    screen.rect(x, 52, 6, 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("chance")
  screen.level(4)
  screen.move(0, 62)
  screen.text("step " .. sel .. " " .. MusicUtil.note_num_to_name(steps[sel].note, true))
  screen.move(127, 62)
  screen.text_right(math.floor(steps[sel].p * 100 + 0.5) .. "%")
  screen.update()
end
