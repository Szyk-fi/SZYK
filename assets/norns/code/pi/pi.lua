-- pi
-- a Portamax norns script
--
-- the digits of pi, worked out at
-- startup with a spigot algorithm
-- (one digit at a time, using only
-- whole numbers), played as scale
-- degrees 0-9. the bars count how
-- often each digit has come up.
--
-- E2 speed   E3 scale
-- K2 jump to a random digit
-- K3 pause   pads: root note
-- (params: octave shape, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local NDIG = 800
local DIVS = { 1 / 4, 1 / 3, 1 / 2, 1 }
local digits = {}
local pos = 1
local hist = {}
local root = 50
local scale = {}
local paused = false

-- Rabinowitz-Wagon spigot: pi as a mixed-radix number, every
-- position holding 2, repeatedly multiplied by ten with carries
local function spigot(n)
  local len = (10 * n) // 3 + 1
  local a = {}
  for i = 1, len do a[i] = 2 end
  local out, nines, pre = {}, 0, nil
  for _ = 1, n do
    local q = 0
    for i = len, 1, -1 do
      local x = 10 * a[i] + q * i
      a[i] = x % (2 * i - 1)
      q = x // (2 * i - 1)
    end
    a[1] = q % 10
    q = q // 10
    if q == 9 then nines = nines + 1
    elseif q == 10 then
      out[#out + 1] = (pre or 0) + 1
      for _ = 1, nines do out[#out + 1] = 0 end
      pre, nines = 0, 0
    else
      if pre then out[#out + 1] = pre end
      pre = q
      for _ = 1, nines do out[#out + 1] = 9 end
      nines = 0
    end
  end
  return out
end

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(root, params:get("scale"), 10)
end

local function reset_hist() for d = 0, 9 do hist[d] = 0 end end

local function play()
  local d = digits[pos]
  hist[d] = hist[d] + 1
  local note = scale[d + 1]
  local up = params:get("shape") == 2 and (pos % 8 >= 4) and 12 or 0
  engine.amp(d == digits[pos - 1] and 0.32 or 0.22)
  engine.pan(util.linlin(0, 9, -0.6, 0.6, d))
  engine.hz(MusicUtil.note_num_to_freq(note + up))
  -- the start of each group of eight gets a low root under it
  if pos % 8 == 1 then
    engine.amp(0.2)
    engine.hz(MusicUtil.note_num_to_freq(root - 12))
  end
end

function init()
  digits = spigot(NDIG)
  reset_hist()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("PI")
  params:add_option("speed", "speed", { "1/16", "1/8t", "1/8", "1/4" }, 3)
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_option("shape", "octave shape", { "flat", "lift every 4" }, 2)
  params:add_control("release", "release", controlspec.new(0.1, 3, 'exp', 0, 0.8, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(2600)
  engine.pw(0.45)
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 10 build_scale() end
  end
  clock.run(function()
    while true do
      if not paused then
        play()
        pos = pos % #digits + 1
        if pos == 1 then reset_hist() end
      end
      redraw()
      clock.sync(DIVS[params:get("speed")])
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("speed", -d)
  elseif n == 3 then params:delta("scale", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then pos = math.random(1, #digits) reset_hist()
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  screen.level(15)
  screen.move(0, 8)
  screen.text("pi")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right("digit " .. pos)
  -- the stream of digits, the one playing boxed in the middle
  local cur = pos - 1
  for i = -6, 6 do
    local k = cur + i
    if k >= 1 and k <= #digits then
      screen.level(i == 0 and 15 or math.max(1, 8 - math.abs(i)))
      screen.move(61 + i * 9, 22)
      screen.text(tostring(digits[k]))
    end
  end
  screen.level(8)
  screen.rect(58, 14, 10, 11)
  screen.stroke()
  -- how often each digit has shown up so far
  local most = 1
  for d = 0, 9 do most = math.max(most, hist[d]) end
  for d = 0, 9 do
    local h = hist[d] / most * 22
    screen.level(cur >= 1 and digits[cur] == d and 15 or 4)
    screen.rect(5 + d * 12, 52 - h, 8, h + 1)
    screen.fill()
  end
  screen.level(4)
  screen.move(0, 62)
  screen.text(params:string("speed"))
  screen.move(127, 62)
  screen.text_right(paused and "paused" or params:string("scale"))
  screen.update()
end
