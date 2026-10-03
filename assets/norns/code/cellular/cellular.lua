-- cellular
-- a Portamax norns script
--
-- a one-dimensional cellular
-- automaton grows down the screen.
-- each new row plays its living
-- cells as a chord, a few at a time.
--
-- E2 rule   E3 voices
-- K2 reseed   K3 single seed
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W = 32
local rows = {}
local scale = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), W)
end

local function seed(single)
  local r = {}
  for i = 1, W do r[i] = single and (i == W // 2 and 1 or 0) or (math.random() < 0.3 and 1 or 0) end
  rows = { r }
end

local function next_row(prev, rule)
  local r = {}
  for i = 1, W do
    local l = prev[(i - 2) % W + 1]
    local c = prev[i]
    local rt = prev[i % W + 1]
    local idx = l * 4 + c * 2 + rt
    r[i] = (rule >> idx) & 1
  end
  return r
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("CELLULAR")
  params:add_number("rule", "rule", 0, 255, 90)
  params:add_number("voices", "voices", 1, 6, 3)
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 24, 60, 36, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.release(1.2)
  engine.cutoff(1800)
  engine.amp(0.15)
  math.randomseed(os.time())
  build_scale()
  seed(true)
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      grow()
      redraw()
    end
  end)
end

function grow()
  local r = next_row(rows[#rows], params:get("rule"))
  table.insert(rows, r)
  while #rows > 14 do table.remove(rows, 1) end
  local alive = {}
  for i = 1, W do if r[i] == 1 then alive[#alive + 1] = i end end
  if #alive == W or #alive == 0 then seed(false) return end
  for v = 1, math.min(params:get("voices"), #alive) do
    local i = alive[math.random(#alive)]
    engine.pan(i / (W / 2) - 1)
    engine.hz(MusicUtil.note_num_to_freq(scale[i]))
  end
end

function enc(n, d)
  if n == 2 then params:delta("rule", d)
  elseif n == 3 then params:delta("voices", d) end
  redraw()
end

function key(n, z)
  if z == 1 then seed(n == 3) redraw() end
end

function redraw()
  screen.clear()
  for y, r in ipairs(rows) do
    screen.level(y == #rows and 15 or 2 + y // 2)
    for x = 1, W do
      if r[x] == 1 then
        screen.rect((x - 1) * 4, 10 + (y - 1) * 3, 3, 2)
        screen.fill()
      end
    end
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("cellular")
  screen.level(6)
  screen.move(127, 7)
  screen.text_right("rule " .. params:get("rule"))
  screen.update()
end
