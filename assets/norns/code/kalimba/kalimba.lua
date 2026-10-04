-- kalimba
-- a Portamax norns script
--
-- a thumb piano. the longest, lowest
-- tine sits in the middle and the
-- scale climbs outward, alternating
-- left and right. it plays a little
-- pattern by itself; the pads pluck
-- tines too (pad 1 = centre).
--
-- E2 pattern   E3 tone
-- K2 new pattern   K3 auto on/off
-- (params: key, sustain, swing)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local NT = 13
local tines = {}   -- tines[slot] = note, slot 1 = leftmost
local order = {}   -- order[k] = slot of the k-th scale degree
local glow = {}
local pattern = {}
local step = 0
local auto = true
local SHAPES = { "thumbs", "climb", "ripple", "scatter" }

local function build()
  local s = MusicUtil.generate_scale_of_length(params:get("key"), "Major", NT)
  local mid = (NT + 1) // 2
  for k = 1, NT do
    local off = (k // 2) * ((k % 2 == 0) and -1 or 1)
    order[k] = mid + off
    tines[order[k]] = s[k]
  end
end

local function new_pattern()
  local shape = SHAPES[params:get("shape")]
  pattern = {}
  for i = 1, 8 do
    local k
    if shape == "thumbs" then k = (i % 2 == 1) and math.random(1, 5) or math.random(4, 9)
    elseif shape == "climb" then k = ((i - 1) * 2) % 9 + 1
    elseif shape == "ripple" then k = ({ 1, 3, 5, 8, 5, 3, 1, 6 })[i]
    else k = math.random(1, NT) end
    pattern[i] = math.random() < 0.12 and 0 or k
  end
  pattern[1] = 1
end

local function pluck(slot, amp)
  engine.pan((slot - (NT + 1) / 2) / NT * 1.5)
  engine.amp(amp)
  engine.hz(MusicUtil.note_num_to_freq(tines[slot]))
  glow[slot] = 15
end

function init()
  params:add_separator("KALIMBA")
  params:add_number("key", "key", 55, 72, 60, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("key", build)
  params:add_option("shape", "pattern", SHAPES, 1)
  params:set_action("shape", new_pattern)
  params:add_control("tone", "tone", controlspec.new(800, 6000, 'exp', 0, 2400, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:add_control("sustain", "sustain", controlspec.new(0.2, 2.5, 'lin', 0, 0.9, 's'))
  params:set_action("sustain", function(x) engine.release(x) end)
  params:add_control("swing", "swing", controlspec.new(0, 0.3, 'lin', 0, 0.12, ''))
  params:default()
  engine.pw(0.5)
  for i = 1, NT do glow[i] = 0 end
  math.randomseed(os.time())
  build()
  new_pattern()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      -- white keys from middle C map onto scale degrees 1, 2, 3...
      local deg = ({ 1, 1, 2, 2, 3, 4, 4, 5, 5, 6, 6, 7 })[msg.note % 12 + 1]
      local k = util.clamp(deg + 7 * ((msg.note - 60) // 12), 1, NT)
      pluck(order[k], 0.3)
    end
  end
  clock.run(function()
    while true do
      local off = (step % 2 == 1) and params:get("swing") / 2 or 0
      clock.sync(1 / 2, off)
      step = step % 8 + 1
      local k = pattern[step]
      if auto and k > 0 then pluck(order[k], step % 4 == 1 and 0.28 or 0.2) end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 20)
      for i = 1, NT do glow[i] = math.max(0, glow[i] - 1) end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("shape", d)
  elseif n == 3 then params:delta("tone", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_pattern()
  elseif n == 3 then auto = not auto end
end

function redraw()
  screen.clear()
  local mid = (NT + 1) // 2
  for slot = 1, NT do
    local len = 40 - math.abs(slot - mid) * 4.5
    local x = 12 + (slot - 1) * 8
    screen.level(math.max(3, glow[slot]))
    screen.rect(x, 14, 5, len)
    screen.fill()
  end
  screen.level(2)
  screen.rect(8, 20, 112, 2)
  screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text("kalimba")
  screen.level(4)
  screen.move(0, 62)
  screen.text(SHAPES[params:get("shape")] .. (auto and "" or " (off)"))
  screen.move(127, 62)
  screen.text_right(MusicUtil.note_num_to_name(params:get("key")) .. " major")
  screen.update()
end
