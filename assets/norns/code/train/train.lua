-- train
-- a Portamax norns script
--
-- the rhythm of the rails. every
-- axle clicks over every rail joint,
-- picking out the current chord.
-- at each station the train slows,
-- a bell rings, and the chord moves on.
--
-- E2 speed   E3 stations apart
-- K2 depart / next chord   K3 pause
-- pads: the whistle
-- (params: root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local STATIONS = { "Ashby", "Brook", "Calder", "Dunmore", "Elm Hill", "Fenwick", "Glen", "Harlow" }
local PROG = { { 0, "minor 7" }, { 5, "minor 7" }, { 10, "dominant 7" }, { 3, "major 7" },
  { 8, "major 7" }, { 2, "diminished" }, { 7, "dominant 7" }, { 0, "minor" } }
local AXLES = { 0, 5, 26, 31, 40, 45, 66, 71 }
local JOINT = 30

local dist, v = 0, 0
local state = "run"
local timer = 0
local stop_at = 0
local idx = 1
local chord = {}
local paused = false
local click = 0

local function build()
  local p = PROG[idx]
  chord = MusicUtil.generate_chord(params:get("root") + p[1], p[2], 0)
end

local function hz(n) engine.hz(MusicUtil.note_num_to_freq(n)) end

function init()
  params:add_separator("TRAIN")
  params:add_number("root", "root", 33, 52, 40, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build)
  params:add_control("speed", "speed", controlspec.new(15, 80, 'lin', 1, 40, ''))
  params:add_number("apart", "stations apart", 8, 64, 24)
  params:add_control("release", "click release", controlspec.new(0.05, 0.8, 'lin', 0, 0.18, 's'))
  params:default()
  engine.cutoff(1400)
  build()
  v = params:get("speed")
  stop_at = params:get("apart")
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      engine.release(1.2) engine.amp(0.2) engine.pw(0.5)
      hz(chord[3] + 24) hz(chord[1] + 36)
    end
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      if not paused then step(1 / 30) end
      click = math.max(0, click - 1)
      redraw()
    end
  end)
end

local function bell()
  engine.release(2.5) engine.amp(0.16) engine.pw(0.5) engine.pan(0.3)
  hz(chord[(timer // 10) % #chord + 1] + 24)
end

function step(dt)
  local before = dist
  local joints = math.floor(dist / JOINT)
  if state == "run" then
    v = v + (params:get("speed") - v) * 0.05
    if joints >= stop_at - 3 then state = "brake" end
  elseif state == "brake" then
    v = math.max(6, v * 0.985)
    if joints >= stop_at then state = "stop" v = 0 timer = 0 end
  elseif state == "stop" then
    timer = timer + 1
    if timer % 10 == 1 and timer < 50 then bell() end
    if timer > 75 then depart() end
  end
  dist = dist + v * dt
  for i, off in ipairs(AXLES) do
    if math.floor((before - off) / JOINT) ~= math.floor((dist - off) / JOINT) then
      engine.release(params:get("release")) engine.amp(0.13 + (i % 2) * 0.06)
      engine.pw(0.2) engine.pan(-0.3)
      hz(chord[(i - 1) % #chord + 1] - (i % 2 == 1 and 12 or 0))
      click = 2
    end
  end
end

function depart()
  idx = idx % #PROG + 1
  build()
  state = "run"
  v = math.max(v, 8)
  stop_at = math.floor(dist / JOINT) + params:get("apart")
  engine.release(1.5) engine.amp(0.18) engine.pan(0)
  hz(chord[1] + 12)
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("apart", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then depart()
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  -- poles and sleepers scroll past at different depths
  screen.level(2)
  for i = 0, 4 do
    local x = 128 - ((dist * 0.5 + i * 32) % 160)
    screen.move(x, 18) screen.line(x, 38) screen.stroke()
  end
  screen.level(3)
  for i = 0, 16 do
    local x = 128 - ((dist * 2 + i * 9) % 144)
    screen.rect(x, 50, 2, 3) screen.fill()
  end
  screen.level(6)
  screen.move(0, 49) screen.line(128, 49) screen.stroke()
  local left = stop_at * JOINT - dist
  if left < 200 then
    local px = 30 + left * 0.5
    screen.level(5)
    screen.rect(px, 44, 50, 4) screen.fill()
    screen.move(px, 41) screen.text(STATIONS[(idx % #STATIONS) + 1])
  end
  screen.level(click > 0 and 15 or 11)
  screen.rect(8, 38, 22, 9) screen.fill()
  screen.rect(32, 38, 22, 9) screen.fill()
  screen.rect(56, 36, 16, 11) screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "train (paused)" or "train")
  screen.move(127, 8)
  screen.text_right(state == "stop" and STATIONS[(idx % #STATIONS) + 1] or "")
  screen.level(4)
  screen.move(0, 62)
  screen.text(MusicUtil.note_num_to_name(chord[1], false) .. " " .. PROG[idx][2])
  screen.move(127, 62)
  screen.text_right(math.floor(v) .. " km/h")
  screen.update()
end
