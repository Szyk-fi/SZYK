-- springs
-- a Portamax norns script
--
-- six masses hang from a beam on
-- springs of different stiffness,
-- each weakly tied to its
-- neighbours. whenever a mass
-- swings back through its rest
-- point it rings, louder the
-- faster it is moving.
--
-- E2 damping    E3 choose mass
-- K2 pluck it   K3 knock the beam
-- pads: pluck mass by note
-- (params: scale, root, breeze)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local N = 6
local BEAM = 12
local REST = 38
local SUB = 4
local masses = {}
local scale = {}
local sel = 1
local t = 0
local beam_shake = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 12)
end

local function new_masses()
  for i = 1, N do
    -- stiffness rises across the row; mass varies a little, so the
    -- natural frequencies sqrt(k/m) are all different
    local m = 0.8 + math.random() * 0.5
    local k = (12 + i * 8) * m * (0.85 + math.random() * 0.3)
    masses[i] = { m = m, k = k, y = 0, v = 0, deg = i * 2 - 1, glow = 0 }
  end
end

local function ring(i, speed)
  local amp = util.clamp(speed * 0.012, 0, 0.32)
  if amp < 0.03 then return end
  local ms = masses[i]
  engine.pan((i - (N + 1) / 2) / N * 1.6)
  engine.amp(amp)
  engine.pw(0.25 + 0.1 * (i % 3))
  engine.release(0.4 + amp * 3)
  engine.cutoff(params:get("bright") * (0.5 + amp * 2))
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(ms.deg, 1, #scale)]))
  ms.glow = 15
end

local function pluck(i, amount)
  masses[i].y = masses[i].y + amount
  masses[i].v = 0
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("SPRINGS")
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("damping", "damping", controlspec.new(0.01, 1.5, 'exp', 0, 0.12, ''))
  params:add_control("coupling", "coupling", controlspec.new(0, 4, 'lin', 0, 0.8, ''))
  params:add_control("breeze", "breeze", controlspec.new(0, 1, 'lin', 0, 0.3, ''))
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 2000, 'hz'))
  params:default()
  engine.gain(1.3)
  math.randomseed(os.time())
  build_scale()
  new_masses()
  -- start with the row pulled down unevenly, so it rings straight away
  for i = 1, N do pluck(i, (i % 2 == 0 and -1 or 1) * (6 + i * 1.5)) end
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local i = (msg.note % N) + 1
      pluck(i, 14)
      sel = i
    end
  end
  local m = metro.init(step, 1 / 60)
  m:start()
end

function step()
  local dt = 1 / 60 / SUB
  local c = params:get("damping")
  local kc = params:get("coupling")
  for _ = 1, SUB do
    for i = 1, N do
      local ms = masses[i]
      local f = -ms.k * ms.y - c * ms.v * ms.m
      if i > 1 then f = f + kc * (masses[i - 1].y - ms.y) end
      if i < N then f = f + kc * (masses[i + 1].y - ms.y) end
      ms.a = f / ms.m
    end
    for i = 1, N do
      local ms = masses[i]
      local before = ms.y
      -- semi-implicit euler: stable for undamped springs at this step
      ms.v = ms.v + ms.a * dt
      ms.y = ms.y + ms.v * dt
      if (before < 0 and ms.y >= 0) or (before > 0 and ms.y <= 0) then
        ring(i, math.abs(ms.v))
      end
    end
  end
  t = t + 1 / 60
  -- an occasional breath of wind nudges a random mass
  if math.random() < params:get("breeze") * 0.012 then
    pluck(math.random(1, N), (math.random() - 0.5) * 20)
  end
  for i = 1, N do masses[i].glow = math.max(0, masses[i].glow - 0.5) end
  beam_shake = beam_shake * 0.85
  if math.floor(t * 60) % 2 == 0 then redraw() end
end

function enc(n, d)
  if n == 2 then params:delta("damping", d)
  elseif n == 3 then sel = util.clamp(sel + d, 1, N) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    pluck(sel, 16)
  elseif n == 3 then
    -- the whole beam jolts: every mass gets the same kick of velocity
    for i = 1, N do masses[i].v = masses[i].v + 60 end
    beam_shake = 3
  end
end

local function x_of(i) return 10 + (i - 1) * 21.6 end

function redraw()
  screen.clear()
  screen.line_width(1)
  local by = BEAM + math.sin(t * 40) * beam_shake
  screen.level(8)
  screen.rect(2, by - 2, 124, 2)
  screen.fill()
  screen.level(1)
  screen.move(0, REST)
  screen.line(128, REST)
  screen.stroke()
  for i, ms in ipairs(masses) do
    local x = x_of(i)
    local y = REST + util.clamp(ms.y, -22, 22)
    -- zigzag spring from beam to mass
    screen.level(i == sel and 10 or 4)
    screen.move(x, by)
    local coils = 8
    local top = y - 4
    for c = 1, coils do
      local yy = by + (top - by) * c / coils
      screen.line(x + ((c % 2 == 0) and -3 or 3), yy)
    end
    screen.line(x, top)
    screen.stroke()
    screen.level(math.max(i == sel and 9 or 5, math.floor(ms.glow)))
    screen.rect(x - 4, y - 4, 8, 7)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 63)
  screen.text("springs")
  screen.level(4)
  screen.move(127, 63)
  screen.text_right("damp " .. params:string("damping"))
  screen.update()
end
