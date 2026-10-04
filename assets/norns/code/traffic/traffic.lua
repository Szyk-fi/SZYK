-- traffic
-- a Portamax norns script
--
-- a crossroads with traffic lights.
-- cars queue at red and roll on
-- green; each car plays its note as
-- it crosses the middle. east-west
-- cars sing high, north-south low.
--
-- E2 traffic   E3 light cycle
-- K2 flip the lights   K3 pause
-- pads: send a car through
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local cars = {}
local scale = {}
local green_ew = true
local phase = 0
local paused = false
local flash = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 15)
end

local function spawn(dir, deg)
  -- dir 1 = east, 2 = west, 3 = south, 4 = north
  local c = { dir = dir, deg = deg or math.random(1, 7), v = 0, crossed = false }
  if dir == 1 then c.x, c.y = -4, 36
  elseif dir == 2 then c.x, c.y = 132, 30
  elseif dir == 3 then c.x, c.y = 60, 8
  else c.x, c.y = 68, 60 end
  cars[#cars + 1] = c
end

local function cross(c)
  local ew = c.dir <= 2
  local n = scale[util.clamp(c.deg + (ew and 7 or 0), 1, #scale)]
  engine.amp(ew and 0.2 or 0.26)
  engine.pw(ew and 0.4 or 0.15)
  engine.pan(ew and (c.dir == 1 and 0.4 or -0.4) or 0)
  engine.hz(MusicUtil.note_num_to_freq(n))
  flash = 3
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("TRAFFIC")
  params:add_option("scale", "scale", names, 8)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("traffic", "traffic", 1, 10, 5)
  params:add_control("cycle", "light cycle", controlspec.new(2, 12, 'lin', 0.5, 5, 's'))
  params:add_control("release", "release", controlspec.new(0.1, 2, 'lin', 0, 0.6, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(2000)
  math.randomseed(os.time())
  build_scale()
  for d = 1, 2 do spawn(d) cars[#cars].x = (d == 1) and 50 or 78 end
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      spawn(math.random(4), (msg.note - 60) % 8 + 1)
      cross(cars[#cars])
    end
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      if not paused then step(1 / 30) end
      flash = math.max(0, flash - 1)
      redraw()
    end
  end)
end

local function pos(c) return (c.dir == 1 and c.x) or (c.dir == 2 and -c.x) or (c.dir == 3 and c.y) or -c.y end
local function stop_line(c) return (c.dir == 1 and 52) or (c.dir == 2 and -76) or (c.dir == 3 and 22) or -46 end

function step(dt)
  phase = phase + dt
  if phase > params:get("cycle") then phase = 0 green_ew = not green_ew end
  if math.random() < params:get("traffic") * 0.01 then spawn(math.random(4)) end
  for i = #cars, 1, -1 do
    local c = cars[i]
    local ew = c.dir <= 2
    local green = (ew == green_ew)
    local p = pos(c)
    local target = 1.2
    if not green and p < stop_line(c) and p > stop_line(c) - 6 then target = 0 end
    -- keep a gap behind the car ahead
    for _, o in ipairs(cars) do
      if o ~= c and o.dir == c.dir then
        local gap = pos(o) - p
        if gap > 0 and gap < 9 then target = 0 end
      end
    end
    c.v = c.v + (target - c.v) * 0.15
    local dx = ({ 1, -1, 0, 0 })[c.dir] * c.v
    local dy = ({ 0, 0, 1, -1 })[c.dir] * c.v
    c.x, c.y = c.x + dx, c.y + dy
    if not c.crossed and p > stop_line(c) + 6 then c.crossed = true cross(c) end
    if c.x < -8 or c.x > 136 or c.y < 0 or c.y > 68 then table.remove(cars, i) end
  end
end

function enc(n, d)
  if n == 2 then params:delta("traffic", d)
  elseif n == 3 then params:delta("cycle", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then green_ew = not green_ew phase = 0 spawn(green_ew and 1 or 3) cross(cars[#cars])
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  screen.level(2)
  screen.rect(0, 26, 128, 14) screen.fill()
  screen.rect(57, 10, 14, 44) screen.fill()
  screen.level(flash > 0 and 6 or 0)
  screen.rect(58, 27, 12, 12) screen.fill()
  for _, c in ipairs(cars) do
    screen.level(c.crossed and 6 or 13)
    if c.dir <= 2 then screen.rect(c.x - 3, c.y - 1, 6, 3) else screen.rect(c.x - 1, c.y - 3, 3, 6) end
    screen.fill()
  end
  screen.level(green_ew and 15 or 3)
  screen.circle(52, 44, 2) screen.fill()
  screen.level(green_ew and 3 or 15)
  screen.circle(76, 22, 2) screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "traffic (paused)" or "traffic")
  screen.level(4)
  screen.move(0, 62)
  screen.text("cars " .. params:get("traffic"))
  screen.move(127, 62)
  screen.text_right("cycle " .. params:string("cycle"))
  screen.update()
end
