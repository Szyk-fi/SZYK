-- geyser
-- a Portamax norns script
--
-- deep water heats under rock.
-- as pressure builds, a slow drone
-- climbs the scale; at the
-- breaking point the geyser erupts
-- in a rush of fast notes, then
-- the chamber drains and refills.
--
-- E2 heat       E3 brightness
-- K2 erupt now  K3 burst shape
-- pads: stoke the chamber
-- (params: scale, root, shape)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local pressure = 0.35
local state = "build" -- build, erupt, refill
local timer = 0
local spray = {}
local bubbles = {}
local next_drone = 0
local t = 0
local shapes = { "rise", "fall", "scatter" }
local burst_id = nil

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 24)
end

local function note(deg, amp, pan, rel, pw, cut)
  engine.pan(pan)
  engine.amp(amp)
  engine.pw(pw)
  engine.release(rel)
  engine.cutoff(cut)
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(math.floor(deg), 1, #scale)]))
end

local function drone()
  -- the drone's degree follows the pressure up through two octaves
  local deg = 1 + math.floor(pressure * 12)
  local b = params:get("bright")
  note(deg, 0.22, -0.3, 2.6, 0.5, b * 0.35)
  note(deg + 4, 0.1, 0.3, 2.2, 0.5, b * 0.3)
end

local function erupt()
  if state == "erupt" then return end
  state = "erupt"
  timer = 0
  local shape = params:get("shape")
  local strength = math.max(0.6, pressure)
  if burst_id then clock.cancel(burst_id) end
  burst_id = clock.run(function()
    local n = math.floor(14 + strength * 18)
    for i = 1, n do
      local deg
      if shape == 1 then deg = 6 + (i % 12) + math.floor(i / 6)
      elseif shape == 2 then deg = #scale - (i % 12) - math.floor(i / 8)
      else deg = math.random(6, #scale) end
      local fade = 1 - i / (n + 4)
      note(deg, 0.08 + 0.18 * fade, (math.random() - 0.5) * 1.6, 0.25 + fade * 0.4, 0.2 + math.random() * 0.3, params:get("bright") * (0.6 + fade))
      clock.sleep(0.045 + (1 - fade) * 0.06)
    end
    burst_id = nil
  end)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("GEYSER")
  params:add_option("scale", "scale", names, 8)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 28, 52, 38, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("heat", "heat", controlspec.new(0.02, 0.5, 'exp', 0, 0.09, '/s'))
  params:add_option("shape", "burst shape", shapes, 1)
  params:add_control("bright", "brightness", controlspec.new(300, 9000, 'exp', 0, 3000, 'hz'))
  params:default()
  engine.gain(1.3)
  math.randomseed(os.time())
  build_scale()
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      pressure = math.min(1.2, pressure + 0.08)
      note(1 + (msg.note % 12), 0.15, 0, 1.2, 0.4, params:get("bright") * 0.5)
    end
  end
  drone()
  next_drone = 1.6
  local m = metro.init(step, 1 / 30)
  m:start()
end

function step()
  local dt = 1 / 30
  t = t + dt
  timer = timer + dt
  if state == "build" then
    -- heat in, with a little turbulence
    pressure = pressure + params:get("heat") * dt * (0.7 + math.random() * 0.6)
    next_drone = next_drone - dt
    if next_drone <= 0 then
      drone()
      next_drone = 2.4 - pressure * 1.6
    end
    if math.random() < pressure * 0.3 then
      table.insert(bubbles, { x = 64 + (math.random() - 0.5) * 30, y = 60, v = 0.3 + pressure * 0.5 })
    end
    if pressure > 1 + math.random() * 0.15 then erupt() end
  elseif state == "erupt" then
    pressure = pressure - dt * 0.4
    for _ = 1, 4 do
      table.insert(spray, { x = 64 + (math.random() - 0.5) * 3, y = 40, vx = (math.random() - 0.5) * 1.6, vy = -3 - math.random() * 3 * math.max(0.3, pressure) })
    end
    if pressure <= 0.05 then
      pressure = 0.05
      state = "refill"
      timer = 0
    end
  elseif state == "refill" then
    -- the chamber drips full again; a quiet low pulse marks it
    if timer > 2 then
      state = "build"
      next_drone = 0
    elseif math.random() < 0.04 then
      note(math.random(1, 5), 0.06, (math.random() - 0.5), 0.5, 0.7, 900)
    end
  end
  for i = #spray, 1, -1 do
    local s = spray[i]
    s.vy = s.vy + 0.18
    s.x = s.x + s.vx
    s.y = s.y + s.vy
    if s.y > 42 then table.remove(spray, i) end
  end
  for i = #bubbles, 1, -1 do
    local b = bubbles[i]
    b.y = b.y - b.v
    b.x = b.x + (64 - b.x) * 0.03
    if b.y < 44 then table.remove(bubbles, i) end
  end
  if #spray > 160 then for _ = 1, 20 do table.remove(spray, 1) end end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("heat", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    pressure = math.max(pressure, 0.8)
    erupt()
  elseif n == 3 then
    params:set("shape", util.wrap(params:get("shape") + 1, 1, #shapes))
    note(8 + params:get("shape") * 3, 0.15, 0, 0.6, 0.3, params:get("bright"))
  end
end

function redraw()
  screen.clear()
  screen.line_width(1)
  -- ground and vent
  screen.level(4)
  screen.move(0, 42)
  screen.line(58, 42)
  screen.line(61, 44)
  screen.move(67, 44)
  screen.line(70, 42)
  screen.line(128, 42)
  screen.stroke()
  -- the chamber, filling with pressure
  screen.level(2)
  screen.rect(34, 46, 60, 16)
  screen.stroke()
  local fill = math.floor(util.clamp(pressure, 0, 1.2) / 1.2 * 15)
  screen.level(state == "erupt" and 12 or 6)
  screen.rect(35, 61 - fill, 58, fill)
  screen.fill()
  screen.level(3)
  screen.move(64, 46)
  screen.line(64, 43)
  screen.stroke()
  for _, b in ipairs(bubbles) do
    screen.level(10)
    screen.pixel(math.floor(b.x), math.floor(b.y))
    screen.fill()
  end
  for _, s in ipairs(spray) do
    screen.level(s.vy < 0 and 15 or 7)
    screen.pixel(math.floor(s.x), math.floor(s.y))
    screen.fill()
  end
  -- steam wisps at rest
  if state ~= "erupt" then
    screen.level(3)
    for i = 0, 2 do
      local y = 38 - i * 5 - (t * 6 % 5)
      screen.pixel(math.floor(64 + math.sin(t * 2 + i) * 2), math.floor(y))
      screen.fill()
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("geyser")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right(state .. " " .. shapes[params:get("shape")])
  screen.move(0, 63)
  screen.text(string.format("%d%%", math.floor(pressure * 100)))
  screen.update()
end
