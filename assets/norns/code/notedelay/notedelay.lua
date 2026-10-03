-- notedelay
-- a Portamax norns script
--
-- a delay made of notes, not audio.
-- each note that comes in (pads or
-- MIDI) is replayed a number of
-- times, each echo softer and
-- shifted in pitch, so a single
-- touch becomes a falling spiral.
-- idle, it plays a seed melody.
--
-- E2 transpose   E3 decay
-- K2 new seed   K3 snap echoes to key
-- (params: repeats, echo time)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = MusicUtil.generate_scale_of_length(36, "Major", 42)
local seed = {}
local sstep = 0
local snap = true
local last_in = -10
local dots = {}

local function voice(n, vel)
  n = util.clamp(n, 24, 108)
  if snap then n = MusicUtil.snap_note_to_array(n, scale) end
  engine.amp(0.3 * vel)
  engine.cutoff(600 + 3000 * vel)
  engine.hz(MusicUtil.note_num_to_freq(n))
  table.insert(dots, { n = n, v = vel, age = 0 })
end

local function echo(note, vel)
  voice(note, vel)
  clock.run(function()
    local n, v = note, vel
    for _ = 1, params:get("repeats") do
      clock.sync(params:get("time"))
      n = n + params:get("transpose")
      v = v * params:get("decay")
      if v < 0.04 then return end
      voice(n, v)
    end
  end)
end

local function new_seed()
  seed = {}
  for i = 1, 8 do
    seed[i] = (i == 1 or math.random() < 0.3) and scale[math.random(22, 30)] or 0
  end
end

function init()
  params:add_separator("NOTEDELAY")
  params:add_number("transpose", "transpose", -12, 12, -5)
  params:add_control("decay", "decay", controlspec.new(0.3, 0.95, 'lin', 0, 0.72, ''))
  params:add_number("repeats", "repeats", 1, 12, 6)
  params:add_control("time", "echo time", controlspec.new(0.25, 1.5, 'lin', 0.25, 0.75, 'beats'))
  params:default()
  engine.release(0.8)
  engine.pw(0.4)
  math.randomseed(os.time())
  new_seed()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      last_in = util.time()
      echo(msg.note, msg.vel / 127)
    end
  end
  clock.run(function()
    while true do
      clock.sync(1)
      sstep = sstep % #seed + 1
      if util.time() - last_in > 2 and seed[sstep] > 0 then echo(seed[sstep], 0.9) end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 15)
      for i = #dots, 1, -1 do
        dots[i].age = dots[i].age + 1
        if dots[i].age > 90 then table.remove(dots, i) end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("transpose", d)
  elseif n == 3 then params:delta("decay", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_seed()
  elseif n == 3 then snap = not snap end
end

function redraw()
  screen.clear()
  for _, d in ipairs(dots) do
    local x = 124 - d.age * 1.3
    local y = 54 - (d.n - 36) * 0.75
    screen.level(math.max(1, math.floor(15 * d.v)))
    screen.circle(x, util.clamp(y, 12, 54), 1 + 2 * d.v)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("notedelay")
  screen.move(127, 8)
  screen.text_right(snap and "in key" or "free")
  screen.level(4)
  screen.move(0, 62)
  local t = params:get("transpose")
  screen.text("shift " .. (t > 0 and "+" or "") .. t)
  screen.move(127, 62)
  screen.text_right("decay " .. string.format("%.2f", params:get("decay")))
  screen.update()
end
