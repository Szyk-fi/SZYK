-- citylights
-- a Portamax norns script
--
-- a skyline at night. windows switch
-- on and off as people come home and
-- go to bed. every window is a note:
-- across the city is the scale, up
-- the towers is the octave.
--
-- E2 activity   E3 brightness
-- K2 new skyline   K3 pause
-- pads: light up a whole floor
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local towers = {}
local scale = {}
local paused = false
local hour = 19
local last = nil

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 24)
end

local function new_skyline()
  towers = {}
  local x = 2
  while x < 120 do
    local w = math.random(3, 5)
    local h = math.random(3, 9)
    local t = { x = x, w = w, h = h, lit = {} }
    for i = 1, w * h do t.lit[i] = math.random() < 0.25 end
    towers[#towers + 1] = t
    x = x + w * 3 + math.random(1, 3)
  end
end

local function window_note(ti, col, row)
  local t = towers[ti]
  local deg = math.floor((t.x + col * 3) / 128 * 7) + 1 + (row // 3) * 7
  return scale[util.clamp(deg, 1, #scale)]
end

local function play(ti, col, row, amp)
  local n = window_note(ti, col, row)
  engine.amp(amp)
  engine.pan(util.linlin(0, 128, -0.7, 0.7, towers[ti].x))
  engine.hz(MusicUtil.note_num_to_freq(n))
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("CITYLIGHTS")
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("activity", "activity", 1, 10, 5)
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 1800, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:add_control("release", "release", controlspec.new(0.1, 3, 'lin', 0, 1.1, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.pw(0.3)
  math.randomseed(os.time())
  build_scale()
  new_skyline()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local row = (msg.note - 60) % 9
      engine.amp(0.12)
      for ti, t in ipairs(towers) do
        if row < t.h then
          for c = 0, t.w - 1 do t.lit[row * t.w + c + 1] = true end
          if ti % 2 == 1 then play(ti, 0, row, 0.14) end
        end
      end
    end
  end
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      if not paused then flick() end
      redraw()
    end
  end)
end

function flick()
  hour = (hour + 0.02) % 24
  local tries = math.random(0, 1) + (math.random(10) <= params:get("activity") and 1 or 0)
  for _ = 1, tries do
    local ti = math.random(#towers)
    local t = towers[ti]
    local i = math.random(#t.lit)
    local row, col = (i - 1) // t.w, (i - 1) % t.w
    -- evenings fill up, small hours empty out
    local want_on = (hour > 17 or hour < 1) and 0.65 or 0.3
    local on = math.random() < want_on
    if on and not t.lit[i] then
      t.lit[i] = true
      last = { ti, col, row }
      play(ti, col, row, 0.22)
    elseif not on and t.lit[i] then
      t.lit[i] = false
      if math.random() < 0.3 then play(ti, col, row, 0.07) end
    end
  end
end

function enc(n, d)
  if n == 2 then params:delta("activity", d)
  elseif n == 3 then params:delta("bright", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_skyline() play(1, 0, 0, 0.2)
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  for ti, t in ipairs(towers) do
    screen.level(1)
    screen.rect(t.x - 1, 54 - t.h * 4, t.w * 3 + 1, t.h * 4 + 1)
    screen.fill()
    for i, on in ipairs(t.lit) do
      local row, col = (i - 1) // t.w, (i - 1) % t.w
      local hot = last and last[1] == ti and last[2] == col and last[3] == row
      screen.level(hot and 15 or (on and 8 or 0))
      screen.rect(t.x + col * 3, 52 - row * 4, 2, 2)
      screen.fill()
    end
  end
  screen.level(3)
  screen.move(0, 55) screen.line(128, 55) screen.stroke()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "citylights (paused)" or "citylights")
  screen.level(6)
  screen.move(127, 8)
  screen.text_right(string.format("%02d:%02d", math.floor(hour), math.floor((hour % 1) * 60)))
  screen.level(4)
  screen.move(0, 62)
  screen.text("activity " .. params:get("activity"))
  screen.move(127, 62)
  screen.text_right(params:string("bright"))
  screen.update()
end
