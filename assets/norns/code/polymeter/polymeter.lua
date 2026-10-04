-- polymeter
-- a Portamax norns script
--
-- two loops of different lengths
-- share one clock: a bass line of
-- five steps against a melody of
-- seven. they line up again every
-- thirty-five steps.
--
-- E2 bass length   E3 melody length
-- K2 new notes   K3 swap roles
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local bass, mel = {}, {}
local bpos, mpos = 0, 0
local count = 0
local scale = {}
local swapped = false

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 14)
end

local function fill(t, len, lo, hi)
  for i = 1, len do t[i] = t[i] or math.random(lo, hi) end
  for i = len + 1, #t do t[i] = nil end
end

local function renew()
  bass, mel = {}, {}
  fill(bass, params:get("blen"), 1, 5)
  fill(mel, params:get("mlen"), 6, 14)
end

local function gcd(a, b) while b ~= 0 do a, b = b, a % b end return a end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("POLYMETER")
  params:add_number("blen", "bass length", 2, 12, 5)
  params:set_action("blen", function(x) fill(bass, x, 1, 5) end)
  params:add_number("mlen", "melody length", 2, 12, 7)
  params:set_action("mlen", function(x) fill(mel, x, 6, 14) end)
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 40, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.amp(0.28)
  math.randomseed(os.time())
  build_scale()
  renew()
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      count = count + 1
      bpos = bpos % #bass + 1
      mpos = mpos % #mel + 1
      local low, high = bass[bpos], mel[mpos]
      if swapped then low, high = high - 5, low + 5 end
      engine.release(0.25)
      engine.cutoff(700)
      engine.pan(-0.4)
      engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(low, 1, 14)]))
      if count % 2 == 0 then
        engine.release(0.6)
        engine.cutoff(2600)
        engine.pan(0.4)
        engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(high, 1, 14)] + 12))
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("blen", d)
  elseif n == 3 then params:delta("mlen", d) end
  bpos, mpos = bpos % #bass, mpos % #mel
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then renew() elseif n == 3 then swapped = not swapped end
  redraw()
end

function redraw()
  screen.clear()
  for i, v in ipairs(bass) do
    screen.level(i == bpos and 15 or 4)
    screen.rect(4 + (i - 1) * 10, 54 - v * 2, 8, v * 2)
    screen.fill()
  end
  for i, v in ipairs(mel) do
    screen.level(i == mpos and 15 or 4)
    screen.rect(4 + (i - 1) * 10, 30 - (v - 6) * 2, 8, 3)
    screen.fill()
  end
  local cycle = #bass * #mel // gcd(#bass, #mel)
  screen.level(15)
  screen.move(0, 7)
  screen.text("polymeter " .. #bass .. ":" .. #mel)
  screen.level(4)
  screen.move(127, 7)
  screen.text_right((count % cycle) + 1 .. "/" .. cycle)
  screen.update()
end
