-- candle
-- a Portamax norns script
--
-- a single candle in a quiet room.
-- the flame wavers on small draughts;
-- each sudden flicker sounds a soft
-- note, taller flame for higher notes,
-- and the brightness of every note
-- follows the brightness of the flame.
--
-- E2 draught   E3 warmth
-- K2 light a new candle   K3 pause
-- pads: a breath across the flame
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local flame, target, lean = 1, 1, 0
local last_note = 0
local wax = 30
local paused = false
local t = 0
local gust = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 12)
end

local function soft(n, amp)
  engine.cutoff(params:get("warmth") * (0.5 + flame * 0.7))
  engine.amp(amp) engine.pw(0.5) engine.pan(lean * 0.3)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("CANDLE")
  params:add_option("scale", "scale", names, 2)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 45, 69, 57, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("draught", "draught", controlspec.new(0, 1, 'lin', 0, 0.35, ''))
  params:add_control("warmth", "warmth", controlspec.new(400, 4000, 'exp', 0, 1400, 'hz'))
  params:add_control("release", "release", controlspec.new(0.5, 6, 'exp', 0, 2.8, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  math.randomseed(os.time())
  build_scale()
  soft(scale[1] - 12, 0.25)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      gust = 1
      soft(scale[(msg.note - 60) % #scale + 1], 0.22)
    end
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      if not paused then burn() end
      redraw()
    end
  end)
  -- a low note now and then, so the room is never empty
  clock.run(function()
    while true do
      if not paused then soft(scale[1] - 12, 0.16) end
      clock.sleep(3 + math.random() * 2)
    end
  end)
end

function burn()
  t = t + 1 / 30
  local d = params:get("draught")
  -- the flame chases a target that jumps on small draughts
  if math.random() < 0.04 + d * 0.15 + gust * 0.5 then
    target = 1 - math.random() * (0.2 + d * 0.6 + gust * 0.4)
    lean = (math.random() - 0.5) * (d + gust) * 2
  end
  gust = gust * 0.9
  local before = flame
  flame = flame + (target - flame) * 0.25 + (math.random() - 0.5) * 0.02
  lean = lean * 0.95
  target = target + (1 - target) * 0.02
  -- a sharp change in the flame is a flicker you can hear
  local jump = math.abs(flame - before)
  if jump > 0.035 and t - last_note > 0.18 then
    last_note = t
    local deg = util.clamp(math.floor(flame * #scale), 1, #scale)
    soft(scale[deg], util.clamp(0.1 + jump * 2, 0.1, 0.3))
  end
  wax = math.max(8, wax - 0.002)
end

function enc(n, d)
  if n == 2 then params:delta("draught", d)
  elseif n == 3 then params:delta("warmth", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then wax = 30 flame, target = 0.3, 1 soft(scale[5], 0.25)
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  local top = 58 - wax
  local fh = 6 + flame * 10
  local tip = top - 2 - fh
  -- glow around the flame
  for r = 3, 1, -1 do
    screen.level(math.floor(flame * (4 - r) * 1.5))
    screen.circle(64 + lean * 2, top - fh * 0.5, 6 + r * 3 * flame)
    screen.fill()
  end
  screen.level(6)
  screen.rect(56, top, 16, wax) screen.fill()
  screen.level(9)
  screen.rect(60, top, 3, 4) screen.fill()
  screen.level(3)
  screen.move(64, top) screen.line(64, top - 3) screen.stroke()
  screen.level(math.floor(8 + flame * 7))
  screen.circle(64, top - 5, 3) screen.fill()
  screen.move(61, top - 5)
  screen.line(64 + lean * 4, tip)
  screen.line(67, top - 5)
  screen.close() screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "candle (paused)" or "candle")
  screen.level(4)
  screen.move(0, 62)
  screen.text("draught " .. params:string("draught"))
  screen.move(127, 62)
  screen.text_right(params:string("warmth"))
  screen.update()
end
