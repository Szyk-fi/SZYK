-- thunderhead
-- a Portamax norns script
--
-- a storm cell gathers charge.
-- rain patters in soft high
-- notes; when the charge breaks,
-- lightning strikes with a bright
-- chord and the thunder rolls in
-- low, late by its distance.
--
-- E2 rain       E3 charge rate
-- K2 strike now K3 squall on/off
-- pads: patter on that note
-- (params: scale, root, echo)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local drops = {}
local splashes = {}
local bolt = nil
local flash = 0
local charge = 0
local threshold = 1
local squall = false
local t = 0
local puffs = {}
local rumble_lvl = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 22)
end

local function note(deg, amp, pan, rel, pw, cut)
  engine.pan(pan)
  engine.amp(amp)
  engine.pw(pw)
  engine.release(rel)
  engine.cutoff(cut)
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(deg, 1, #scale)]))
end

local function setup_echo()
  audio.level_eng_cut(0.5)
  softcut.buffer_clear()
  softcut.enable(1, 1)
  softcut.buffer(1, 1)
  softcut.level(1, 0.5)
  softcut.pan(1, -0.2)
  softcut.rate(1, 1)
  softcut.loop(1, 1)
  softcut.loop_start(1, 1)
  softcut.loop_end(1, 2.1)
  softcut.position(1, 1)
  softcut.fade_time(1, 0.05)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 1)
  softcut.pre_level(1, 0.5)
  softcut.play(1, 1)
  softcut.rec(1, 1)
  softcut.filter_dry(1, 0)
  softcut.filter_lp(1, 1)
  softcut.filter_fc(1, 1200)
  softcut.filter_rq(1, 2)
end

local function make_bolt()
  local x = 30 + math.random() * 68
  local pts = { { x, 22 } }
  local y = 22
  while y < 60 do
    y = y + 3 + math.random() * 5
    x = x + (math.random() - 0.5) * 12
    table.insert(pts, { x, math.min(60, y) })
  end
  local fork = nil
  if math.random() < 0.6 then
    local k = math.random(2, math.max(2, #pts - 2))
    fork = { { pts[k][1], pts[k][2] } }
    local fx, fy = pts[k][1], pts[k][2]
    for _ = 1, 3 do
      fx = fx + (math.random() < 0.5 and -1 or 1) * (3 + math.random() * 5)
      fy = fy + 4 + math.random() * 4
      table.insert(fork, { fx, fy })
    end
  end
  return { pts = pts, fork = fork, life = 9 }
end

local function strike()
  bolt = make_bolt()
  flash = 15
  charge = 0
  threshold = 0.7 + math.random() * 0.6
  -- the crack: a bright open chord right away
  local top = #scale - math.random(0, 4)
  for i = 0, 2 do note(top - i * 2, 0.16, (i - 1) * 0.6, 1.2, 0.15, 7000) end
  -- thunder arrives later the farther away the strike was
  local dist = math.random()
  clock.run(function()
    clock.sleep(0.15 + dist * 1.4)
    rumble_lvl = 1 - dist * 0.5
    for i = 1, 6 do
      note(math.random(1, 4), (0.42 - i * 0.05) * (1 - dist * 0.4), (math.random() - 0.5), 2.5, 0.5, 300 + math.random() * 300)
      clock.sleep(0.09 + math.random() * 0.2)
    end
  end)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("THUNDERHEAD")
  params:add_option("scale", "scale", names, 12)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 24, 48, 33, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("rain", "rain", controlspec.new(0.5, 30, 'exp', 0, 6, '/s'))
  params:add_control("charge", "charge rate", controlspec.new(0.02, 1, 'exp', 0, 0.12, '/s'))
  params:add_control("echo", "echo", controlspec.new(0, 0.9, 'lin', 0, 0.5, ''))
  params:set_action("echo", function(x) softcut.pre_level(1, x) end)
  params:default()
  engine.gain(1.4)
  setup_echo()
  math.randomseed(os.time())
  build_scale()
  for i = 1, 9 do puffs[i] = { x = 6 + i * 13 + math.random(-3, 3), r = 7 + math.random() * 6 } end
  charge = 0.45
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      note(14 + (msg.note % 8), 0.14, (math.random() - 0.5), 0.25, 0.3, 5000)
      table.insert(drops, { x = math.random() * 128, y = 22, v = 3 })
    end
  end
  -- the storm opens with a first drop on the roof
  note(16, 0.12, 0.2, 0.3, 0.3, 5000)
  local m = metro.init(step, 1 / 30)
  m:start()
end

function step()
  local dt = 1 / 30
  t = t + dt
  local rain = params:get("rain") * (squall and 2 or 1)
  local slant = squall and 1.2 or 0.2
  if math.random() < rain * dt * 2 then
    table.insert(drops, { x = math.random() * 140 - 12, y = 20 + math.random() * 6, v = 2.5 + math.random() * 1.5 })
  end
  for i = #drops, 1, -1 do
    local d = drops[i]
    d.y = d.y + d.v
    d.x = d.x + slant
    if d.y >= 62 then
      table.remove(drops, i)
      table.insert(splashes, { x = d.x, life = 4 })
      -- only some drops are heard, so the patter stays a texture
      if math.random() < 0.5 then
        note(13 + math.random(0, 8), 0.05 + math.random() * 0.06, (d.x / 128 - 0.5) * 1.4, 0.12, 0.6, 6000)
      end
    end
  end
  for i = #splashes, 1, -1 do
    splashes[i].life = splashes[i].life - 1
    if splashes[i].life <= 0 then table.remove(splashes, i) end
  end
  -- charge separates faster while it rains hard
  charge = charge + params:get("charge") * dt * (0.6 + rain / 15) * (0.5 + math.random())
  if charge >= threshold then strike() end
  if bolt then
    bolt.life = bolt.life - 1
    if bolt.life <= 0 then bolt = nil end
  end
  flash = math.max(0, flash - 2)
  rumble_lvl = rumble_lvl * 0.95
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("rain", d)
  elseif n == 3 then params:delta("charge", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then strike()
  elseif n == 3 then squall = not squall end
end

function redraw()
  screen.clear()
  screen.line_width(1)
  if flash > 9 then
    screen.level(3)
    screen.rect(0, 0, 128, 64)
    screen.fill()
  end
  local shake = math.floor(rumble_lvl * 2 * math.sin(t * 50))
  -- the cloud, darker as it charges
  for _, p in ipairs(puffs) do
    screen.level(util.clamp(math.floor(10 - charge * 6) + (flash > 0 and 4 or 0), 2, 15))
    screen.circle(p.x + shake, 13 + math.sin(t * 0.5 + p.x) * 1.5, p.r)
    screen.fill()
  end
  screen.level(0)
  screen.rect(0, 0, 128, 3)
  screen.fill()
  if bolt then
    screen.level(15)
    screen.move(bolt.pts[1][1], bolt.pts[1][2])
    for i = 2, #bolt.pts do screen.line(bolt.pts[i][1], bolt.pts[i][2]) end
    screen.stroke()
    if bolt.fork then
      screen.level(8)
      screen.move(bolt.fork[1][1], bolt.fork[1][2])
      for i = 2, #bolt.fork do screen.line(bolt.fork[i][1], bolt.fork[i][2]) end
      screen.stroke()
    end
  end
  screen.level(6)
  for _, d in ipairs(drops) do
    screen.move(d.x, d.y)
    screen.line(d.x - 1, d.y - 3)
    screen.stroke()
  end
  for _, s in ipairs(splashes) do
    screen.level(s.life * 2)
    screen.pixel(math.floor(s.x) - 1, 62)
    screen.pixel(math.floor(s.x) + 1, 62)
    screen.fill()
  end
  -- charge meter
  screen.level(2)
  screen.rect(122, 26, 4, 34)
  screen.stroke()
  screen.level(10)
  local h = math.floor(util.clamp(charge / threshold, 0, 1) * 32)
  screen.rect(123, 59 - h, 2, h)
  screen.fill()
  screen.level(squall and 15 or 7)
  screen.move(0, 36)
  screen.text(squall and "thunderhead: squall" or "thunderhead")
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
