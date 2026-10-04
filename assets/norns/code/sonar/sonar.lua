-- sonar
-- a Portamax norns script
--
-- a beam sweeps the dark and
-- pings. whatever it touches
-- answers late and low if it is
-- far, soon and high if near,
-- and the water rings on.
--
-- E2 sweep speed  E3 range
-- K2 new contacts K3 hold beam
-- pads: drop a contact
-- (params: key, water echo)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local CX, CY, R = 40, 32, 30
local beam = 0               -- radians
local contacts = {}
local blips = {}             -- persistence of echoes on the screen
local pending = {}           -- echoes on their way back
local pulses = {}            -- expanding ping fronts
local hold = false
local ping_t = 0
local scale = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("key"), "Phrygian", 22)
end

local function add_contact(r, a)
  table.insert(contacts, {
    r = r or (0.2 + math.random() * 0.75),
    a = a or (math.random() * 2 * math.pi),
    size = 0.4 + math.random() * 0.6,
    drift = (math.random() - 0.5) * 0.04,
    kind = math.random(3),
  })
end

local function new_contacts()
  contacts = {}
  for _ = 1, 7 do add_contact() end
end

local function setup_water()
  -- the engine feeds a long, darkened softcut loop: the sea's reverb
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  for v = 1, 2 do
    softcut.enable(v, 1)
    softcut.buffer(v, v)
    softcut.level(v, 0.5)
    softcut.pan(v, v == 1 and -0.7 or 0.7)
    softcut.rate(v, 1)
    softcut.loop(v, 1)
    softcut.loop_start(v, 1)
    softcut.loop_end(v, 1 + (v == 1 and 0.61 or 0.83))
    softcut.position(v, 1)
    softcut.fade_time(v, 0.05)
    softcut.level_input_cut(1, v, 0.7)
    softcut.level_input_cut(2, v, 0.7)
    softcut.rec_level(v, 1)
    softcut.pre_level(v, params:get("water"))
    softcut.play(v, 1)
    softcut.rec(v, 1)
    softcut.filter_dry(v, 0)
    softcut.filter_lp(v, 1)
    softcut.filter_fc(v, 1100)
    softcut.filter_rq(v, 2)
  end
end

local function ping()
  -- the outgoing ping: a high, glassy note
  engine.pan(0)
  engine.pw(0.5)
  engine.cutoff(7000)
  engine.release(0.5)
  engine.amp(0.22)
  engine.hz(MusicUtil.note_num_to_freq(params:get("key") + 36))
  table.insert(pulses, { a = beam, r = 0 })
  -- which contacts sit inside the beam's width?
  for _, c in ipairs(contacts) do
    local da = math.abs(((c.a - beam) + math.pi) % (2 * math.pi) - math.pi)
    if da < 0.35 then
      -- the echo returns after the round trip; far is late and low
      local delay = c.r * params:get("range") * 2
      table.insert(pending, { t = delay, c = c, strength = (1 - da / 0.35) * c.size })
    end
  end
end

local function echo(e)
  local c = e.c
  local deg = util.clamp(math.floor(util.linlin(0.15, 1, 21, 1, c.r)), 1, 22)
  local n = scale[deg]
  engine.pan(util.clamp(math.cos(c.a) * 0.8, -0.8, 0.8))
  engine.pw(({ 0.5, 0.25, 0.1 })[c.kind])
  engine.cutoff(util.linlin(0, 1, 3500, 900, c.r))
  engine.release(0.4 + c.size * 1.4)
  engine.amp(0.25 * e.strength + 0.05)
  engine.hz(MusicUtil.note_num_to_freq(n))
  table.insert(blips, { x = CX + math.cos(c.a) * c.r * R, y = CY + math.sin(c.a) * c.r * R, life = 30, size = c.size })
end

local function step(dt)
  if not hold then beam = (beam + dt * params:get("sweep")) % (2 * math.pi) end
  ping_t = ping_t - dt
  if ping_t <= 0 then
    ping()
    ping_t = params:get("interval")
  end
  for i = #pending, 1, -1 do
    pending[i].t = pending[i].t - dt
    if pending[i].t <= 0 then
      echo(pending[i])
      table.remove(pending, i)
    end
  end
  for i = #pulses, 1, -1 do
    pulses[i].r = pulses[i].r + dt / params:get("range")
    if pulses[i].r > 1 then table.remove(pulses, i) end
  end
  for i = #blips, 1, -1 do
    blips[i].life = blips[i].life - 1
    if blips[i].life <= 0 then table.remove(blips, i) end
  end
  -- contacts drift slowly about the water
  for _, c in ipairs(contacts) do
    c.a = c.a + c.drift * dt
    c.r = util.clamp(c.r + math.sin(util.time() * 0.1 + c.a) * 0.002 * dt * 10, 0.12, 0.98)
  end
end

function init()
  params:add_separator("SONAR")
  params:add_number("key", "key", 36, 60, 45, function(p) return MusicUtil.note_num_to_name(p:get(), false) end)
  params:set_action("key", build_scale)
  params:add_control("sweep", "sweep", controlspec.new(0.1, 3, 'exp', 0, 0.7, 'rad/s'))
  params:add_control("range", "range", controlspec.new(0.3, 3, 'exp', 0, 1.2, 's'))
  params:add_control("interval", "ping every", controlspec.new(0.25, 3, 'exp', 0, 0.9, 's'))
  params:add_control("water", "water echo", controlspec.new(0, 0.9, 'lin', 0, 0.55, ''))
  params:set_action("water", function(x) softcut.pre_level(1, x) softcut.pre_level(2, x) end)
  params:default()
  engine.gain(1)
  math.randomseed(os.time())
  build_scale()
  new_contacts()
  setup_water()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      -- a contact right in the beam, its range set by the pad
      add_contact(util.linlin(60, 72, 0.9, 0.2, msg.note), beam + 0.05)
      if #contacts > 14 then table.remove(contacts, 1) end
    end
  end
  local last = util.time()
  local frame = metro.init(function()
    local now = util.time()
    step(math.min(0.1, now - last))
    last = now
    redraw()
  end, 1 / 30)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("sweep", d)
  elseif n == 3 then params:delta("range", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_contacts()
  elseif n == 3 then hold = not hold end
  redraw()
end

function redraw()
  screen.clear()
  -- range rings
  for k = 1, 3 do
    screen.level(k == 3 and 3 or 1)
    screen.circle(CX, CY, R * k / 3)
    screen.stroke()
  end
  -- the beam and its fading wake
  for k = 0, 6 do
    local a = beam - k * 0.08
    screen.level(math.max(1, 12 - k * 2))
    screen.move(CX, CY)
    screen.line(CX + math.cos(a) * R, CY + math.sin(a) * R)
    screen.stroke()
  end
  -- ping fronts travelling out along the beam
  for _, p in ipairs(pulses) do
    screen.level(math.floor(10 * (1 - p.r)) + 2)
    screen.arc(CX, CY, p.r * R, p.a - 0.35, p.a + 0.35)
    screen.stroke()
  end
  -- echoes
  for _, b in ipairs(blips) do
    screen.level(math.min(15, math.floor(b.life / 2)))
    screen.circle(b.x, b.y, 1 + b.size * 1.5)
    screen.fill()
  end
  -- readout
  screen.level(15)
  screen.move(127, 8)
  screen.text_right("sonar")
  screen.level(5)
  screen.move(127, 22)
  screen.text_right(string.format("brg %03d", math.floor(math.deg(beam)) % 360))
  screen.move(127, 32)
  screen.text_right(#contacts .. " contacts")
  screen.move(127, 42)
  screen.text_right(string.format("rng %.1fs", params:get("range")))
  if hold then
    screen.level(12)
    screen.move(127, 62)
    screen.text_right("hold")
  end
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
  softcut.rec(2, 0)
end
