-- steps
-- a Portamax norns script
--
-- a plain, friendly eight-step
-- sequencer. each step has a note
-- and can be on or off.
--
-- E1 tempo
-- E2 choose step   E3 step's note
-- K2 step on/off   K3 random
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local seq = {}
local on = {}
local sel = 1
local playhead = 0
local scale = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 15)
end

local function randomize()
  for i = 1, 8 do
    seq[i] = math.random(1, 15)
    on[i] = math.random() < 0.75
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("STEPS")
  params:add_option("scale", "scale", names, 2)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("release", "release", controlspec.new(0.05, 2, 'exp', 0, 0.3, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:add_control("cutoff", "cutoff", controlspec.new(200, 8000, 'exp', 0, 1600, 'hz'))
  params:set_action("cutoff", function(x) engine.cutoff(x) end)
  params:default()
  engine.amp(0.3)
  build_scale()
  for i = 1, 8 do seq[i] = ({ 1, 3, 5, 8, 5, 3, 6, 4 })[i] on[i] = true end
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      playhead = playhead % 8 + 1
      if on[playhead] then engine.hz(MusicUtil.note_num_to_freq(scale[seq[playhead]])) end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 1 then params:delta("clock_tempo", d)
  elseif n == 2 then sel = util.clamp(sel + d, 1, 8)
  elseif n == 3 then seq[sel] = util.clamp(seq[sel] + d, 1, 15) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then on[sel] = not on[sel] elseif n == 3 then randomize() end
  redraw()
end

function redraw()
  screen.clear()
  for i = 1, 8 do
    local x = 4 + (i - 1) * 15
    local h = seq[i] * 2.6
    screen.level(i == playhead and 15 or (on[i] and 6 or 2))
    screen.rect(x, 54 - h, 11, h)
    if on[i] then screen.fill() else screen.stroke() end
    if i == sel then
      screen.level(15)
      screen.rect(x, 58, 11, 2)
      screen.fill()
    end
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("steps")
  screen.level(4)
  screen.move(127, 7)
  screen.text_right(params:get("clock_tempo") .. " bpm  " .. MusicUtil.note_num_to_name(scale[seq[sel]], true))
  screen.update()
end
