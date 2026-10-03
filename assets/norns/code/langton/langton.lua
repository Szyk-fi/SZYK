-- langton
-- a Portamax norns script
--
-- a few ants wander a small
-- toroidal field. on a light cell
-- an ant turns right and steps up
-- the scale; on a dark cell it
-- turns left and steps down. the
-- colour it leaves picks octave.
--
-- E2 speed   E3 brightness
-- K2 clear field   K3 add ant
-- pads: drop an ant
-- (params: scale, root, ants max)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local COLS, ROWS, CELL = 32, 13, 4
local field = {}
local ants = {}
local scale = {}
local turn = 0
local steps = 0
local DX = { 0, 1, 0, -1 }
local DY = { -1, 0, 1, 0 }

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 22)
end

local function clear_field()
  for y = 1, ROWS do
    field[y] = {}
    for x = 1, COLS do field[y][x] = 0 end
  end
  steps = 0
end

local function add_ant(x, y)
  if #ants >= params:get("max_ants") then table.remove(ants, 1) end
  table.insert(ants, {
    x = x or math.random(1, COLS),
    y = y or math.random(1, ROWS),
    dir = math.random(1, 4),
    degree = math.random(4, 9),
    flash = 0,
  })
end

local function play_ant(i, a, light)
  local deg = a.degree + (light and 7 or 0)
  local note = scale[util.clamp(deg, 1, #scale)]
  engine.pan(util.linlin(1, COLS, -0.9, 0.9, a.x))
  engine.pw(light and 0.25 or 0.55)
  engine.amp(light and 0.2 or 0.26)
  engine.hz(MusicUtil.note_num_to_freq(note))
  a.flash = 15
end

local function advance()
  steps = steps + 1
  turn = turn % #ants + 1
  for i, a in ipairs(ants) do
    local c = field[a.y][a.x]
    if c == 0 then
      a.dir = a.dir % 4 + 1
      a.degree = a.degree + 1
    else
      a.dir = (a.dir + 2) % 4 + 1
      a.degree = a.degree - 1
    end
    -- keep each ant's melody inside a singable octave and a half
    if a.degree > 11 then a.degree = 5 elseif a.degree < 1 then a.degree = 7 end
    field[a.y][a.x] = 1 - c
    -- one ant sings per step, the rest walk silently
    if i == turn then play_ant(i, a, c == 0) end
    a.x = (a.x + DX[a.dir] - 1) % COLS + 1
    a.y = (a.y + DY[a.dir] - 1) % ROWS + 1
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("LANGTON")
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("max_ants", "ants max", 1, 6, 4)
  params:add_option("speed", "speed", { "1/2", "1/4", "1/8", "1/16" }, 2)
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 1500, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:add_control("release", "release", controlspec.new(0.05, 3, 'exp', 0, 0.5, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.gain(1.6)
  math.randomseed(os.time())
  build_scale()
  clear_field()
  for _ = 1, 3 do add_ant() end
  advance()

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      add_ant((msg.note * 5) % COLS + 1, (msg.note * 3) % ROWS + 1)
      local a = ants[#ants]
      engine.pan(0)
      engine.hz(MusicUtil.note_num_to_freq(msg.note))
      a.flash = 15
    end
  end

  clock.run(function()
    local divs = { 1 / 2, 1 / 4, 1 / 8, 1 / 16 }
    while true do
      clock.sync(divs[params:get("speed")])
      advance()
    end
  end)
  local mt = metro.init(function()
    for _, a in ipairs(ants) do a.flash = math.max(0, a.flash - 1) end
    redraw()
  end, 1 / 20)
  mt:start()
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then clear_field()
  elseif n == 3 then add_ant() end
end

function redraw()
  screen.clear()
  screen.level(3)
  for y = 1, ROWS do
    local row = field[y]
    for x = 1, COLS do
      if row[x] == 1 then
        screen.rect((x - 1) * CELL, (y - 1) * CELL, CELL - 1, CELL - 1)
        screen.fill()
      end
    end
  end
  for _, a in ipairs(ants) do
    screen.level(math.max(9, a.flash))
    local cx, cy = (a.x - 1) * CELL + 1.5, (a.y - 1) * CELL + 1.5
    screen.rect(cx - 1, cy - 1, 3, 3)
    screen.fill()
  end
  screen.level(1)
  screen.move(0, ROWS * CELL + 1)
  screen.line(127, ROWS * CELL + 1)
  screen.stroke()
  screen.level(15)
  screen.move(0, 62)
  screen.text("langton")
  screen.level(5)
  screen.move(44, 62)
  screen.text(#ants .. " ants")
  screen.move(127, 62)
  screen.text_right(steps .. " st")
  screen.update()
end
