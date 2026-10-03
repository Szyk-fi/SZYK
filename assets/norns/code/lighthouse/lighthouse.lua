-- lighthouse
-- a Portamax norns script
--
-- a lighthouse beam turns over a
-- dark sea. ships ride at
-- different distances; as the
-- beam finds one it sounds a note
-- (near ships high and clear, far
-- ones low and soft). fog shortens
-- the beam and hides far ships.
--
-- E2 rotation   E3 fog
-- K2 new fleet  K3 foghorn on/off
-- pads: a ship sails in
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local CX, CY = 64, 36
local ships = {}
local scale = {}
local angle = 0
local foghorn = false
local t = 0
local mist = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function reach()
  return 12 + (1 - params:get("fog")) * 80
end

local function place(s)
  s.dist = math.sqrt((s.x - CX) ^ 2 + (s.y - CY) ^ 2)
  s.bearing = math.atan(s.y - CY, s.x - CX) % (2 * math.pi)
end

local function new_ship(x, y)
  local s = {
    x = x or math.random(4, 124),
    y = y or math.random(14, 62),
    vx = (math.random() < 0.5 and -1 or 1) * (0.03 + math.random() * 0.08),
    lit = 0,
  }
  place(s)
  if s.dist < 9 then s.x = s.x + 14 place(s) end
  return s
end

local function new_fleet()
  ships = {}
  for _ = 1, 6 do table.insert(ships, new_ship()) end
end

local function sound(s)
  local r = reach()
  local seen = 1 - s.dist / r
  if seen <= 0 then return end
  -- distance chooses the pitch: the nearest ships sit highest
  local deg = util.clamp(math.floor(util.linlin(8, 80, #scale, 1, s.dist)), 1, #scale)
  engine.pan(util.clamp((s.x - CX) / 64, -1, 1))
  engine.amp(0.08 + 0.24 * seen)
  engine.pw(0.3)
  engine.release(0.5 + (1 - seen) * 2)
  engine.cutoff(400 + 5000 * seen * seen)
  engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
  s.lit = 15
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("LIGHTHOUSE")
  params:add_option("scale", "scale", names, 10)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 64, 43, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("speed", "rotation", controlspec.new(0.05, 2, 'exp', 0, 0.35, 'rev/s'))
  params:add_control("fog", "fog", controlspec.new(0, 1, 'lin', 0, 0.3, ''))
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  build_scale()
  new_fleet()
  -- one ship is right in the beam's path as it starts to turn
  ships[1] = new_ship(CX + 20, CY + 6)
  angle = (ships[1].bearing - 0.15) % (2 * math.pi)
  for i = 1, 40 do mist[i] = { x = math.random() * 128, y = 12 + math.random() * 52 } end
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      if #ships > 10 then table.remove(ships, 1) end
      local s = new_ship()
      table.insert(ships, s)
      sound(s)
    end
  end
  local m = metro.init(step, 1 / 30)
  m:start()
end

function step()
  t = t + 1 / 30
  local before = angle
  angle = angle + params:get("speed") * 2 * math.pi / 30
  local wrapped = angle >= 2 * math.pi
  for _, s in ipairs(ships) do
    s.x = s.x + s.vx
    if s.x < -2 then s.x = 130 elseif s.x > 130 then s.x = -2 end
    place(s)
    local b = s.bearing
    -- did the beam sweep past this ship's bearing this frame?
    if (b > before and b <= angle) or (wrapped and b <= angle - 2 * math.pi) then
      sound(s)
    end
    s.lit = math.max(0, s.lit - 0.7)
  end
  if wrapped then
    angle = angle - 2 * math.pi
    if foghorn then
      engine.pan(0)
      engine.amp(0.3)
      engine.pw(0.5)
      engine.release(3.5)
      engine.cutoff(500)
      engine.hz(MusicUtil.note_num_to_freq(scale[1] - 12))
    end
  end
  for _, m in ipairs(mist) do
    m.x = m.x + 0.15
    if m.x > 128 then m.x = 0 end
  end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("fog", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_fleet()
  elseif n == 3 then foghorn = not foghorn end
end

function redraw()
  screen.clear()
  screen.line_width(1)
  local r = reach()
  local fog = params:get("fog")
  -- fog: drifting specks, more of them the thicker it is
  for i, m in ipairs(mist) do
    if i <= fog * #mist then
      screen.level(2)
      screen.pixel(math.floor(m.x), math.floor(m.y))
      screen.fill()
    end
  end
  -- the beam: a narrow wedge out to the fog's reach
  local w = 0.12
  screen.level(5)
  screen.move(CX, CY)
  screen.line(CX + math.cos(angle - w) * r, CY + math.sin(angle - w) * r)
  screen.line(CX + math.cos(angle + w) * r, CY + math.sin(angle + w) * r)
  screen.close()
  screen.fill()
  screen.level(12)
  screen.move(CX, CY)
  screen.line(CX + math.cos(angle) * r, CY + math.sin(angle) * r)
  screen.stroke()
  for _, s in ipairs(ships) do
    local visible = s.dist < r
    screen.level(math.max(visible and 4 or 1, math.floor(s.lit)))
    local x, y = math.floor(s.x), math.floor(s.y)
    screen.move(x - 3, y)
    screen.line(x + 3, y)
    screen.line(x + 2, y + 2)
    screen.line(x - 2, y + 2)
    screen.close()
    screen.fill()
    screen.move(x, y)
    screen.line(x, y - 3)
    screen.stroke()
  end
  -- the tower
  screen.level(15)
  screen.circle(CX, CY, 2.5)
  screen.fill()
  screen.level(0)
  screen.circle(CX, CY, 1)
  screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(foghorn and "lighthouse  horn" or "lighthouse")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right(string.format("fog %d%%", math.floor(fog * 100)))
  screen.update()
end
