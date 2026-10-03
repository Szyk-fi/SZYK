-- flock
-- a Portamax norns script
--
-- a small flock of birds: each
-- follows its neighbours, keeps
-- its distance, and heads the way
-- the others head. every beat the
-- highest birds sing; height is
-- pitch, left and right is pan.
--
-- E2 cohesion   E3 singers
-- K2 scatter   K3 hawk
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local birds = {}
local scale = {}
local hawk = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 20)
end

local function scatter()
  for _, b in ipairs(birds) do
    b.vx = (math.random() - 0.5) * 4
    b.vy = (math.random() - 0.5) * 4
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("FLOCK")
  params:add_control("cohesion", "cohesion", controlspec.new(0, 0.05, 'lin', 0, 0.01, ''))
  params:add_number("singers", "singers", 1, 4, 2)
  params:add_option("scale", "scale", names, 2)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.release(0.9)
  engine.cutoff(2600)
  engine.amp(0.2)
  math.randomseed(os.time())
  build_scale()
  for i = 1, 12 do
    birds[i] = { x = math.random(20, 108), y = math.random(15, 55), vx = math.random() - 0.5, vy = math.random() - 0.5 }
  end
  local m = metro.init(fly, 1 / 30)
  m:start()
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      sing()
    end
  end)
end

function fly()
  local cx, cy, vx, vy = 0, 0, 0, 0
  for _, b in ipairs(birds) do cx, cy, vx, vy = cx + b.x, cy + b.y, vx + b.vx, vy + b.vy end
  local n = #birds
  cx, cy, vx, vy = cx / n, cy / n, vx / n, vy / n
  local k = params:get("cohesion")
  for i, b in ipairs(birds) do
    b.vx = b.vx + (cx - b.x) * k + (vx - b.vx) * 0.05
    b.vy = b.vy + (cy - b.y) * k + (vy - b.vy) * 0.05
    for j, o in ipairs(birds) do
      if i ~= j then
        local dx, dy = b.x - o.x, b.y - o.y
        local d2 = dx * dx + dy * dy
        if d2 < 36 and d2 > 0 then b.vx = b.vx + dx / d2 b.vy = b.vy + dy / d2 end
      end
    end
    if hawk > 0 then
      local dx, dy = b.x - 64, b.y - 36
      local d = math.sqrt(dx * dx + dy * dy) + 1
      b.vx, b.vy = b.vx + dx / d * 0.4, b.vy + dy / d * 0.4
    end
    local sp = math.sqrt(b.vx * b.vx + b.vy * b.vy)
    if sp > 2.5 then b.vx, b.vy = b.vx / sp * 2.5, b.vy / sp * 2.5 end
    b.x = b.x + b.vx
    b.y = b.y + b.vy
    if b.x < 2 or b.x > 126 then b.vx = -b.vx b.x = util.clamp(b.x, 2, 126) end
    if b.y < 12 or b.y > 62 then b.vy = -b.vy b.y = util.clamp(b.y, 12, 62) end
  end
  hawk = math.max(0, hawk - 1)
  redraw()
end

function sing()
  table.sort(birds, function(a, b) return a.y < b.y end)
  for i = 1, params:get("singers") do
    local b = birds[i]
    b.sang = 6
    local deg = util.clamp(math.floor((62 - b.y) / 50 * 19) + 1, 1, 20)
    engine.pan(b.x / 64 - 1)
    engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
  end
end

function enc(n, d)
  if n == 2 then params:delta("cohesion", d)
  elseif n == 3 then params:delta("singers", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then scatter() elseif n == 3 then hawk = 40 end
end

function redraw()
  screen.clear()
  for _, b in ipairs(birds) do
    screen.level((b.sang or 0) > 0 and 15 or 6)
    b.sang = math.max(0, (b.sang or 0) - 1)
    screen.move(b.x, b.y)
    screen.line(b.x - b.vx * 2, b.y - b.vy * 2)
    screen.stroke()
  end
  if hawk > 0 then
    screen.level(15)
    screen.circle(64, 36, 3)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("flock")
  screen.update()
end
