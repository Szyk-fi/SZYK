-- mandel
-- a Portamax norns script
--
-- a scanner slides along a line
-- through the mandelbrot set. at
-- each point it counts how long
-- z = z*z + c takes to escape: quick
-- escapes are low notes, the slow
-- ones near the edge ring high, and
-- points inside the set rest.
--
-- E2 move line   E3 points per line
-- K2 flip direction   K3 pause
-- pads: root note
-- (params: scale, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local MAXIT = 40
local MX, MY, MW, MH = 0, 12, 64, 44
local X0, X1, Y0, Y1 = -2.2, 0.6, -1.1, 1.1
local map = {}
local samples = {}
local pos = 1
local dir = 1
local root = 43
local scale = {}
local paused = false

local function escape(cr, ci)
  local zr, zi = 0, 0
  for i = 1, MAXIT do
    zr, zi = zr * zr - zi * zi + cr, 2 * zr * zi + ci
    if zr * zr + zi * zi > 4 then return i end
  end
  return MAXIT
end

local function build_map()
  for y = 0, MH - 1 do
    map[y] = {}
    for x = 0, MW - 1 do
      map[y][x] = escape(X0 + (X1 - X0) * x / (MW - 1), Y0 + (Y1 - Y0) * y / (MH - 1))
    end
  end
end

local function scan()
  samples = {}
  local im = params:get("im")
  local n = params:get("points")
  for i = 1, n do samples[i] = escape(X0 + (X1 - X0) * (i - 1) / (n - 1), im) end
  pos = util.clamp(pos, 1, n)
end

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(root, params:get("scale"), 22)
end

local function play(it)
  if it >= MAXIT then return end
  -- escape time grows fast near the edge, so hear it on a log scale
  local deg = math.floor(util.linlin(0, math.log(MAXIT), 1, #scale, math.log(it)))
  engine.amp(util.linlin(1, MAXIT, 0.3, 0.18, it))
  engine.pan(util.linlin(1, #samples, -0.6, 0.6, pos))
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(deg, 1, #scale)]))
end

function init()
  build_map()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("MANDEL")
  params:add_control("im", "line (imag)", controlspec.new(-1.1, 1.1, 'lin', 0.01, 0.62, ''))
  params:set_action("im", scan)
  params:add_number("points", "points", 8, 48, 24)
  params:set_action("points", scan)
  params:add_option("scale", "scale", names, 6)
  params:set_action("scale", build_scale)
  params:add_control("release", "release", controlspec.new(0.1, 3, 'exp', 0, 0.7, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(2400)
  engine.pw(0.4)
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 17 build_scale() end
  end
  clock.run(function()
    while true do
      if not paused then
        play(samples[pos])
        pos = pos + dir
        if pos > #samples then pos = 1 elseif pos < 1 then pos = #samples end
      end
      redraw()
      clock.sync(1 / 4)
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("im", -d)
  elseif n == 3 then params:delta("points", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then dir = -dir
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  for y = 0, MH - 1 do
    local row = map[y]
    for x = 0, MW - 1 do
      local it = row[x]
      if it >= 5 then
        screen.level(it >= MAXIT and 3 or util.clamp(it // 2, 1, 10))
        screen.pixel(MX + x, MY + y)
        screen.fill()
      end
    end
  end
  -- the scan line and the point being heard
  local ly = MY + (params:get("im") - Y0) / (Y1 - Y0) * (MH - 1)
  screen.level(6)
  screen.move(MX, ly)
  screen.line(MX + MW, ly)
  screen.stroke()
  local n = #samples
  local cur = (pos - dir - 1) % n + 1
  screen.level(15)
  screen.circle(MX + (cur - 1) / (n - 1) * (MW - 1), ly, 2)
  screen.fill()
  -- escape times along the line, as bars
  local w = 60 / n
  for i, it in ipairs(samples) do
    local h = it >= MAXIT and 34 or math.log(it) / math.log(MAXIT) * 34
    screen.level(i == cur and 15 or (it >= MAXIT and 2 or 5))
    screen.rect(67 + (i - 1) * w, 52 - h, math.max(1, w - 1), h + 1)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("mandel")
  screen.level(4)
  screen.move(0, 62)
  screen.text(string.format("im %.2f", params:get("im")))
  screen.move(127, 62)
  local it = samples[cur] or 0
  screen.text_right(paused and "paused" or (it >= MAXIT and "inside" or ("esc " .. it)))
  screen.update()
end
