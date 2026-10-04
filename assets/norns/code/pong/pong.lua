-- pong
-- a Portamax norns script
--
-- two paddles rally on their own.
-- every paddle hit is a note, set
-- by where the ball meets it; wall
-- bounces answer more quietly.
-- a miss drops a low tone.
--
-- E2 ball speed   E3 brightness
-- K2 serve   K3 pause
-- pads: root note
-- (params: scale, root, reach)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local TOP, BOT = 12, 54
local ball = { x = 64, y = 30, vx = 1, vy = 1 }
local pads = { 30, 30 }
local scale = {}
local paused = false
local score = { 0, 0 }
local flash = { 0, 0 }

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 15)
end

local function play(deg, amp, pan, rel)
  engine.amp(amp)
  engine.pan(pan)
  engine.release(rel)
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(deg, 1, #scale)]))
end

local function serve(dir)
  ball.x, ball.y = 64, math.random(TOP + 6, BOT - 6)
  local k = params:get("speed")
  ball.vx = (dir or (math.random(2) == 1 and 1 or -1)) * 2.2 * k
  ball.vy = (math.random() * 1.2 + 0.9) * k * (math.random(2) == 1 and 1 or -1)
  play(1, 0.2, 0, 1.2)
end

local function step()
  if paused then redraw() return end
  local reach = params:get("reach")
  for i = 1, 2 do
    -- each paddle drifts toward the ball, but only so fast
    local d = ball.y - pads[i]
    pads[i] = pads[i] + util.clamp(d, -reach, reach)
    pads[i] = util.clamp(pads[i], TOP + 5, BOT - 5)
    flash[i] = math.max(0, flash[i] - 1)
  end
  ball.x, ball.y = ball.x + ball.vx, ball.y + ball.vy
  if ball.y < TOP or ball.y > BOT then
    ball.vy = -ball.vy
    ball.y = util.clamp(ball.y, TOP, BOT)
    play(ball.y < 30 and 8 or 5, 0.1, (ball.x - 64) / 64, 0.4)
  end
  for i, px in ipairs({ 6, 121 }) do
    local coming = (i == 1 and ball.vx < 0 and ball.x <= px) or (i == 2 and ball.vx > 0 and ball.x >= px)
    if coming then
      local off = ball.y - pads[i]
      if math.abs(off) <= 6 then
        ball.vx = -ball.vx
        ball.vy = ball.vy + off * 0.15
        flash[i] = 8
        -- higher on the screen, higher in the scale
        play(math.floor(util.linlin(BOT, TOP, 1, 15, ball.y)), 0.3, i == 1 and -0.6 or 0.6, 1.4)
      else
        score[3 - i] = score[3 - i] + 1
        play(1, 0.25, 0, 2.2)
        serve(i == 1 and 1 or -1)
      end
    end
  end
  redraw()
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("PONG")
  params:add_option("scale", "scale", names, 8)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("speed", "ball speed", controlspec.new(0.4, 3, 'exp', 0, 1, 'x'))
  params:add_control("reach", "paddle reach", controlspec.new(0.5, 4, 'lin', 0, 1.6, 'px'))
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 2400, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  math.randomseed(os.time())
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then params:set("root", msg.note - 12) end
  end
  serve()
  metro.init(step, 1 / 30):start()
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
  if n == 2 then serve() elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  screen.level(2)
  screen.move(0, TOP - 2) screen.line(128, TOP - 2) screen.stroke()
  screen.move(0, BOT + 2) screen.line(128, BOT + 2) screen.stroke()
  for y = TOP, BOT, 4 do screen.pixel(64, y) end
  screen.fill()
  for i, px in ipairs({ 3, 122 }) do
    screen.level(flash[i] > 0 and 15 or 6)
    screen.rect(px, pads[i] - 6, 3, 12)
    screen.fill()
  end
  screen.level(15)
  screen.rect(ball.x - 1, ball.y - 1, 3, 3)
  screen.fill()
  screen.move(0, 8)
  screen.text(paused and "pong (paused)" or "pong")
  screen.move(127, 8)
  screen.text_right(score[1] .. " : " .. score[2])
  screen.level(4)
  screen.move(0, 62)
  screen.text(MusicUtil.note_num_to_name(params:get("root"), true))
  screen.move(127, 62)
  screen.text_right("speed " .. params:string("speed"))
  screen.update()
end
