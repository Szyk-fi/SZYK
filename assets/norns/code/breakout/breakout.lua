-- breakout
-- a Portamax norns script
--
-- a ball chips away at a wall of
-- tuned bricks. columns are scale
-- steps, rows are octaves: the top
-- row rings highest. clear the
-- wall and a new one is built.
--
-- E2 ball speed   E3 brightness
-- K2 new wall   K3 pause
-- pads: change key
-- (params: scale, root, rows)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local COLS, BW, BH, BY = 8, 14, 4, 12
local bricks = {}
local ball = { x = 64, y = 40, vx = 1, vy = -1 }
local paddle = 64
local intervals = {}
local paused = false
local hits = 0
local sparks = {}

local function build_scale()
  intervals = MusicUtil.SCALES[params:get("scale")].intervals
end

local function note(col, row)
  -- column walks the scale, row picks the octave (row 1 is the top)
  local steps = #intervals - 1
  local deg = (col - 1) % steps
  local oct = (col - 1) // steps + (params:get("rows") - row)
  return params:get("root") + 12 * oct + intervals[deg + 1]
end

local function new_wall()
  bricks = {}
  for r = 1, params:get("rows") do
    bricks[r] = {}
    for c = 1, COLS do bricks[r][c] = true end
  end
  ball.x, ball.y = math.random(30, 98), BY + params:get("rows") * (BH + 1) + 6
  local k = params:get("speed")
  ball.vx, ball.vy = (math.random(2) == 1 and 1.3 or -1.3) * k, -1.6 * k
  engine.amp(0.2)
  engine.release(2)
  for i, iv in ipairs({ 0, 7, 12 }) do engine.hz(MusicUtil.note_num_to_freq(params:get("root") + iv)) end
end

local function hit_brick()
  local r = (ball.y - BY) // (BH + 1) + 1
  local c = (ball.x - 4) // (BW + 1) + 1
  r, c = math.floor(r), math.floor(c)
  if bricks[r] and bricks[r][c] then
    bricks[r][c] = false
    ball.vy = -ball.vy
    hits = hits + 1
    engine.amp(0.28)
    engine.release(0.9 + r * 0.3)
    engine.pan((c - 4.5) / 5)
    engine.hz(MusicUtil.note_num_to_freq(note(c, r)))
    sparks[#sparks + 1] = { x = 4 + (c - 1) * (BW + 1) + BW / 2, y = BY + (r - 1) * (BH + 1) + 2, t = 8 }
    for _, row in ipairs(bricks) do
      for _, b in ipairs(row) do if b then return end end
    end
    new_wall()
  end
end

local function step()
  if not paused then
    paddle = paddle + util.clamp(ball.x - paddle, -2.5, 2.5)
    ball.x, ball.y = ball.x + ball.vx, ball.y + ball.vy
    if ball.x < 4 or ball.x > 123 then ball.vx = -ball.vx ball.x = util.clamp(ball.x, 4, 123) end
    if ball.y < BY then ball.vy = math.abs(ball.vy) end
    hit_brick()
    if ball.y >= 53 and ball.vy > 0 then
      if math.abs(ball.x - paddle) < 9 then
        ball.vy = -ball.vy
        ball.vx = util.clamp(ball.vx + (ball.x - paddle) * 0.12, -3, 3)
        engine.amp(0.12)
        engine.release(0.5)
        engine.pan(0)
        engine.hz(MusicUtil.note_num_to_freq(params:get("root") - 12))
      elseif ball.y > 60 then
        ball.x, ball.y, ball.vy = paddle, 50, -math.abs(ball.vy)
      end
    end
  end
  for i = #sparks, 1, -1 do
    sparks[i].t = sparks[i].t - 1
    if sparks[i].t <= 0 then table.remove(sparks, i) end
  end
  redraw()
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("BREAKOUT")
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:add_number("rows", "rows (octaves)", 2, 4, 4)
  params:add_control("speed", "ball speed", controlspec.new(0.5, 2.5, 'lin', 0, 1.2, 'x'))
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 3000, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  math.randomseed(os.time())
  build_scale()
  new_wall()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then params:set("root", msg.note - 24) end
  end
  metro.init(step, 1 / 40):start()
end

function enc(n, d)
  if n == 2 then
    local old = params:get("speed")
    params:delta("speed", d)
    local r = params:get("speed") / old
    ball.vx, ball.vy = ball.vx * r, ball.vy * r
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_wall() elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  for r, row in ipairs(bricks) do
    for c, b in ipairs(row) do
      if b then
        screen.level(16 - r * 3)
        screen.rect(4 + (c - 1) * (BW + 1), BY + (r - 1) * (BH + 1), BW, BH)
        screen.fill()
      end
    end
  end
  for _, s in ipairs(sparks) do
    screen.level(s.t + 6)
    screen.circle(s.x, s.y, 9 - s.t)
    screen.stroke()
  end
  screen.level(15)
  screen.rect(ball.x - 1, ball.y - 1, 2, 2)
  screen.fill()
  screen.level(10)
  screen.rect(paddle - 8, 54, 16, 2)
  screen.fill()
  screen.move(0, 8)
  screen.text(paused and "breakout (paused)" or "breakout")
  screen.level(4)
  screen.move(0, 62)
  screen.text(MusicUtil.note_num_to_name(params:get("root"), false) .. " " .. string.lower(MusicUtil.SCALES[params:get("scale")].name))
  screen.move(127, 62)
  screen.text_right(hits)
  screen.update()
end
