-- spiral
-- a Portamax norns script
--
-- seeds grow out from the middle
-- the way a sunflower packs them,
-- each one turned by the golden
-- angle. the turn picks the note;
-- the distance from the centre
-- picks the octave.
--
-- E2 angle offset   E3 growth
-- K2 restart   K3 golden angle
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local GOLDEN = 137.508
local seeds = {}
local n = 0
local scale = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 7)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("SPIRAL")
  params:add_control("offset", "angle offset", controlspec.new(-5, 5, 'lin', 0.01, 0, 'deg'))
  params:add_number("growth", "seeds per beat", 1, 4, 2)
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.release(1.1)
  engine.amp(0.2)
  engine.cutoff(2600)
  build_scale()
  clock.run(function()
    while true do
      clock.sync(1 / params:get("growth"))
      grow()
      redraw()
    end
  end)
end

function grow()
  n = n + 1
  local angle = n * (GOLDEN + params:get("offset"))
  local r = math.sqrt(n) * 2.2
  if r > 30 then n = 0 seeds = {} return end
  local turn = (angle % 360) / 360
  local deg = math.floor(turn * 7) + 1
  local octave = math.floor(r / 10)
  table.insert(seeds, { a = math.rad(angle), r = r })
  engine.pan(math.cos(math.rad(angle)) * 0.8)
  engine.hz(MusicUtil.note_num_to_freq(scale[deg] + octave * 12))
end

function enc(n_, d)
  if n_ == 2 then params:delta("offset", d)
  elseif n_ == 3 then params:delta("growth", d) end
end

function key(k, z)
  if z == 0 then return end
  if k == 2 then n = 0 seeds = {}
  elseif k == 3 then params:set("offset", 0) end
end

function redraw()
  screen.clear()
  for i, s in ipairs(seeds) do
    screen.level(i == #seeds and 15 or math.max(2, 10 - (#seeds - i) // 8))
    screen.circle(64 + math.cos(s.a) * s.r * 1.6, 34 + math.sin(s.a) * s.r, 1)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("spiral")
  screen.level(4)
  screen.move(127, 7)
  screen.text_right(string.format("%.2f deg", GOLDEN + params:get("offset")))
  screen.update()
end
