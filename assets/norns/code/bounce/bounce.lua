-- bounce
-- a Portamax norns script
--
-- balls bounce around a box.
-- the floor and walls are tuned:
-- where a ball lands decides the
-- note, how fast it hits decides
-- how loud.
--
-- E2 gravity   E3 balls
-- K2 throw   K3 clear
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local balls = {}
local scale = {}
local marks = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function throw()
  table.insert(balls, { x = math.random(10, 118), y = math.random(12, 40), vx = (math.random() - 0.5) * 3, vy = 2 })
  while #balls > params:get("balls") do table.remove(balls, 1) end
end

local function ring(where, speed, pan)
  local deg = util.clamp(math.floor(where * 15) + 1, 1, 16)
  engine.amp(util.clamp(speed / 6, 0.05, 0.4))
  engine.pan(pan)
  engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("BOUNCE")
  params:add_control("gravity", "gravity", controlspec.new(0.02, 0.5, 'exp', 0, 0.15, ''))
  params:add_number("balls", "balls", 1, 8, 3)
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 52, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.release(0.5)
  engine.cutoff(3000)
  math.randomseed(os.time())
  build_scale()
  for _ = 1, 3 do throw() end
  local m = metro.init(step, 1 / 40)
  m:start()
end

function step()
  local g = params:get("gravity")
  for _, b in ipairs(balls) do
    b.vy = b.vy + g
    b.x = b.x + b.vx
    b.y = b.y + b.vy
    if b.y > 60 then
      b.y = 60
      local s = math.abs(b.vy)
      b.vy = -b.vy * 0.92
      if s > 0.6 then ring(b.x / 128, s, b.x / 64 - 1) table.insert(marks, { x = b.x, y = 62, t = 8 }) end
      if s < 0.6 then b.vy = -4 - math.random() * 2 end -- keep it lively
    end
    if b.x < 2 or b.x > 126 then
      b.vx = -b.vx
      b.x = util.clamp(b.x, 2, 126)
      ring(1 - b.y / 64, math.abs(b.vx) * 2, b.x < 64 and -1 or 1)
      table.insert(marks, { x = b.x, y = b.y, t = 8 })
    end
  end
  for i = #marks, 1, -1 do
    marks[i].t = marks[i].t - 1
    if marks[i].t <= 0 then table.remove(marks, i) end
  end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("gravity", d)
  elseif n == 3 then params:delta("balls", d) while #balls < params:get("balls") do throw() end end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then throw() elseif n == 3 then balls = {} end
end

function redraw()
  screen.clear()
  screen.level(3)
  screen.rect(0, 10, 128, 54)
  screen.stroke()
  for _, m in ipairs(marks) do
    screen.level(m.t)
    screen.circle(m.x, m.y, 9 - m.t)
    screen.stroke()
  end
  screen.level(15)
  for _, b in ipairs(balls) do
    screen.circle(b.x, b.y, 2)
    screen.fill()
  end
  screen.move(0, 7)
  screen.text("bounce")
  screen.level(4)
  screen.move(127, 7)
  screen.text_right("g " .. params:string("gravity"))
  screen.update()
end
