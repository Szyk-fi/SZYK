-- seismograph
-- a Portamax norns script
--
-- a pen draws the ground's tremor
-- on scrolling paper. quakes come
-- with gutenberg-richter odds; big
-- ones ring low and dense, and
-- their aftershocks thin out by
-- omori's law, rising and fading.
--
-- E2 activity   E3 brightness
-- K2 quake now  K3 lift pen (pause)
-- pads: a quake sized by the note
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W = 128
local PEN_X = 100
local MID = 36
local trace = {}
local scale = {}
local lifted = false
local t = 0
-- the current sequence: mainshock time/magnitude
local main = nil
local shake = 0.4 -- current trace amplitude (pixels)
local log = {} -- recent events for the readout: { m, age }
local ticks = {} -- event markers drawn on the paper { x, h }

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 20)
end

local function ring(deg, amp, pan, rel)
  deg = util.clamp(math.floor(deg), 1, #scale)
  engine.amp(amp)
  engine.pan(pan)
  engine.release(rel)
  engine.pw(util.clamp(0.15 + amp * 0.6, 0.1, 0.8))
  engine.cutoff(params:get("bright") * (0.4 + amp))
  engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
end

-- a tremor of magnitude m: shakes the pen and plays a burst whose
-- note count grows with magnitude and whose pitch sinks with it.
local function tremor(m, is_main)
  shake = math.max(shake, m * m * 0.7)
  if is_main then main = { t = t, m = m } end
  table.insert(log, 1, { m = m, age = 0 })
  if #log > 4 then table.remove(log) end
  table.insert(ticks, { x = PEN_X, h = math.min(26, m * 4) })
  local count = math.max(1, math.floor(m * 0.9))
  local top = util.clamp(math.floor(#scale - m * 2.4), 2, #scale)
  clock.run(function()
    for i = 1, count do
      local amp = util.clamp(0.08 + m * 0.045, 0.05, 0.4) * (1 - (i - 1) / (count + 1))
      local deg = top - math.random(0, 3) - (i % 2) * 2
      ring(deg, amp, (math.random() - 0.5) * 1.2, 0.3 + m * 0.35)
      clock.sleep(0.05 + math.random() * 0.12 / math.max(1, m - 1))
    end
  end)
end

-- gutenberg-richter: small quakes are common, large ones rare
local function draw_magnitude(lo)
  local u = math.random()
  return lo - math.log(1 - u * 0.999, 10) * 1.1
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("SEISMOGRAPH")
  params:add_option("scale", "scale", names, 6)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 24, 60, 38, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("activity", "activity", controlspec.new(0.05, 2, 'exp', 0, 0.4, '/s'))
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 1800, 'hz'))
  params:add_control("release", "release", controlspec.new(0.5, 4, 'lin', 0, 1, 'x'))
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  build_scale()
  for x = 1, W do trace[x] = MID end
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      tremor(util.clamp((msg.note - 48) / 4, 1.5, 7), true)
    end
  end
  -- the ground opens with a modest quake
  tremor(4.2, true)
  local m = metro.init(step, 1 / 30)
  m:start()
end

function step()
  if lifted then redraw() return end
  local dt = 1 / 30
  t = t + dt
  -- background mainshocks
  if math.random() < params:get("activity") * dt then
    tremor(draw_magnitude(1.5), true)
  end
  -- aftershocks: rate K / (c + dt)^p (omori), smaller than the main
  if main then
    local since = t - main.t
    local rate = (main.m - 1.5) * 1.6 / (0.4 + since) ^ 1.1
    if math.random() < rate * dt then
      local m = main.m - 1.2 - math.random() * 1.5
      if m > 0.8 then tremor(m, false) end
    end
    if since > 40 or rate < 0.02 then main = nil end
  end
  shake = shake * 0.94 + 0.02
  -- scroll the paper and write the pen
  for x = 1, W - 1 do trace[x] = trace[x + 1] end
  local y = MID + (math.random() * 2 - 1) * shake + math.sin(t * 23) * shake * 0.5
  trace[W] = util.clamp(y, 12, 60)
  for i = #ticks, 1, -1 do
    ticks[i].x = ticks[i].x - 1
    if ticks[i].x < 0 then table.remove(ticks, i) end
  end
  for _, e in ipairs(log) do e.age = e.age + dt end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("activity", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then tremor(5 + math.random() * 1.5, true)
  elseif n == 3 then lifted = not lifted end
end

function redraw()
  screen.clear()
  screen.line_width(1)
  -- ruled paper
  screen.level(1)
  for yy = 16, 60, 11 do
    screen.move(0, yy)
    screen.line(PEN_X, yy)
    screen.stroke()
  end
  -- event marks along the top edge of the paper
  for _, k in ipairs(ticks) do
    screen.level(3)
    screen.move(k.x, 12)
    screen.line(k.x, 12 + k.h * 0.25)
    screen.stroke()
  end
  -- the trace, up to the pen
  screen.level(12)
  local off = W - PEN_X
  screen.move(0, trace[1 + off])
  for x = 1, PEN_X - 1 do
    screen.line(x, trace[x + 1 + off])
  end
  screen.stroke()
  -- the pen arm
  local py = trace[W]
  screen.level(lifted and 4 or 15)
  screen.circle(PEN_X, py, 2)
  screen.fill()
  screen.level(5)
  screen.move(PEN_X, py)
  screen.line(127, MID)
  screen.stroke()
  -- readout
  screen.level(15)
  screen.move(0, 8)
  screen.text(lifted and "seismograph (pen up)" or "seismograph")
  for i, e in ipairs(log) do
    screen.level(math.max(2, 12 - math.floor(e.age)))
    screen.move(127, 8 + i * 8)
    screen.text_right(string.format("M%.1f", e.m))
  end
  screen.level(4)
  screen.move(0, 63)
  screen.text("act " .. params:string("activity"))
  screen.update()
end
