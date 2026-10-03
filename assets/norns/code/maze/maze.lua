-- maze
-- a Portamax norns script
--
-- a walker feels its way through a
-- freshly dug maze, one hand on the
-- wall. a right turn steps the
-- melody up, a left turn steps it
-- down, a dead end leaps an octave.
-- reach the exit: a new maze.
--
-- E2 walking speed   E3 brightness
-- K2 new maze   K3 pause
-- pads: move the key
-- (params: scale, root, hand)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W, H, C, X0, Y0 = 20, 7, 6, 4, 12
local DX, DY = { 1, 0, -1, 0 }, { 0, 1, 0, -1 }
local open = {} -- open[x][y][d] = true when the side d of a cell has no wall
local walker = { x = 1, y = 1, d = 1 }
local trail = {}
local scale = {}
local deg = 8
local paused = false
local solved = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 22)
end

local function dig()
  open = {}
  local seen = {}
  for x = 1, W do open[x], seen[x] = {}, {} for y = 1, H do open[x][y] = {} end end
  local stack = { { 1, 1 } }
  seen[1][1] = true
  while #stack > 0 do
    local c = stack[#stack]
    local ways = {}
    for d = 1, 4 do
      local nx, ny = c[1] + DX[d], c[2] + DY[d]
      if nx >= 1 and nx <= W and ny >= 1 and ny <= H and not seen[nx][ny] then ways[#ways + 1] = d end
    end
    if #ways == 0 then
      table.remove(stack)
    else
      local d = ways[math.random(#ways)]
      local nx, ny = c[1] + DX[d], c[2] + DY[d]
      open[c[1]][c[2]][d] = true
      open[nx][ny][(d + 1) % 4 + 1] = true
      seen[nx][ny] = true
      stack[#stack + 1] = { nx, ny }
    end
  end
  walker = { x = 1, y = 1, d = 1 }
  trail = {}
  deg = 8
end

local function play(amp, rel)
  engine.amp(amp)
  engine.release(rel)
  engine.pan((walker.x - W / 2) / W)
  engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
end

local function walk()
  -- try the hand side first, then straight, then the other side, then back
  local hand = params:get("hand") == 1 and 1 or -1
  local tries = { hand, 0, -hand, 2 }
  for _, t in ipairs(tries) do
    local d = (walker.d - 1 + t) % 4 + 1
    if open[walker.x][walker.y][d] then
      if t == 1 then deg = deg + 1
      elseif t == -1 then deg = deg - 1
      elseif t == 2 then deg = deg + (deg > 11 and -7 or 7) end
      deg = util.clamp(deg, 1, #scale)
      walker.d = d
      walker.x, walker.y = walker.x + DX[d], walker.y + DY[d]
      trail[#trail + 1] = { walker.x, walker.y }
      if #trail > 40 then table.remove(trail, 1) end
      play(t == 0 and 0.14 or 0.26, t == 0 and 0.5 or 1.1)
      break
    end
  end
  if walker.x == W and walker.y == H then
    solved = solved + 1
    engine.amp(0.22)
    engine.release(2.5)
    for _, i in ipairs({ 1, 3, 5, 8 }) do engine.hz(MusicUtil.note_num_to_freq(scale[i])) end
    dig()
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("MAZE")
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_option("hand", "hand on wall", { "right", "left" }, 1)
  params:add_number("speed", "steps per beat", 1, 6, 2)
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 1800, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  math.randomseed(os.time())
  build_scale()
  dig()
  play(0.2, 1)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then params:set("root", msg.note - 12) end
  end
  clock.run(function()
    while true do
      clock.sync(1 / params:get("speed"))
      if not paused then walk() end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then dig() elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  screen.level(5)
  screen.line_width(1)
  for x = 1, W do
    for y = 1, H do
      local px, py = X0 + (x - 1) * C, Y0 + (y - 1) * C
      if not open[x][y][1] then screen.move(px + C, py) screen.line(px + C, py + C) end
      if not open[x][y][2] then screen.move(px, py + C) screen.line(px + C, py + C) end
    end
  end
  screen.move(X0, Y0) screen.line(X0 + W * C, Y0)
  screen.move(X0, Y0) screen.line(X0, Y0 + H * C)
  screen.stroke()
  for i, t in ipairs(trail) do
    screen.level(math.max(1, i * 8 // #trail))
    screen.rect(X0 + (t[1] - 1) * C + 2, Y0 + (t[2] - 1) * C + 2, 2, 2)
    screen.fill()
  end
  screen.level(15)
  screen.rect(X0 + (walker.x - 1) * C + 1, Y0 + (walker.y - 1) * C + 1, 4, 4)
  screen.fill()
  screen.level(10)
  screen.rect(X0 + (W - 1) * C + 1, Y0 + (H - 1) * C + 1, 4, 4)
  screen.stroke()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "maze (paused)" or "maze")
  screen.level(4)
  screen.move(0, 62)
  screen.text(MusicUtil.note_num_to_name(scale[deg], true))
  screen.move(127, 62)
  screen.text_right("solved " .. solved)
  screen.update()
end
