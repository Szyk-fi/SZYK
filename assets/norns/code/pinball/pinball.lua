-- pinball
-- a Portamax norns script
--
-- a ball rattles around a table of
-- round bumpers. each bumper is a
-- tone of the chord, so every
-- rally is an arpeggio. the
-- flippers play on their own but
-- K2 flips them too.
--
-- E2 chord   E3 flipper skill
-- K2 flip   K3 tilt (freeze)
-- pads: move the chord root
-- (params: root, gravity, brightness)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local CHORDS = { "major 7", "minor 7", "dominant 7", "sus2", "sus4", "major", "minor" }
local bumpers = {
  { x = 34, y = 24, r = 5 }, { x = 64, y = 18, r = 5 }, { x = 94, y = 24, r = 5 },
  { x = 49, y = 36, r = 4 }, { x = 79, y = 36, r = 4 },
}
local ball = { x = 118, y = 54, vx = -1, vy = -4 }
local flip = 0
local frozen = false
local score = 0
local notes = {}

local function build()
  local c = MusicUtil.generate_chord(params:get("root"), CHORDS[params:get("chord")], 0)
  -- five bumpers: the chord, then its root and third an octave up
  notes = { c[1], c[2], c[3], c[4] or c[1] + 12, c[2] + 12 }
end

local function tone(n, amp, rel, pan)
  engine.amp(amp)
  engine.release(rel)
  engine.pan(pan)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function launch()
  ball.x, ball.y = 118, 54
  ball.vx, ball.vy = -0.6 - math.random(), -3.6 - math.random()
  tone(params:get("root") - 12, 0.25, 1.5, 0.5)
end

local function physics()
  ball.vy = ball.vy + params:get("gravity")
  ball.x, ball.y = ball.x + ball.vx, ball.y + ball.vy
  if ball.x < 5 or ball.x > 123 then ball.vx = -ball.vx * 0.9 ball.x = util.clamp(ball.x, 5, 123) end
  if ball.y < 12 then
    ball.vy = math.abs(ball.vy) * 0.8
    ball.y = 12
    tone(notes[3] + 12, 0.08, 0.3, (ball.x - 64) / 64)
  end
  for i, b in ipairs(bumpers) do
    local dx, dy = ball.x - b.x, ball.y - b.y
    local d = math.sqrt(dx * dx + dy * dy)
    if d < b.r + 1.5 and d > 0 then
      local nx, ny = dx / d, dy / d
      local dot = ball.vx * nx + ball.vy * ny
      if dot < 0 then
        ball.vx, ball.vy = ball.vx - 2 * dot * nx, ball.vy - 2 * dot * ny
        local sp = math.sqrt(ball.vx ^ 2 + ball.vy ^ 2)
        local want = math.max(sp, 2.4)
        ball.vx, ball.vy = ball.vx / sp * want, ball.vy / sp * want
        b.lit = 10
        score = score + 10
        tone(notes[i], 0.3, 1.2, (b.x - 64) / 64)
      end
      ball.x, ball.y = b.x + nx * (b.r + 1.6), b.y + ny * (b.r + 1.6)
    end
  end
  -- the flipper zone
  if ball.y > 51 and ball.vy > 0 and ball.x > 34 and ball.x < 94 then
    if flip == 0 and math.random() < params:get("skill") then flip = 6 end
    if flip > 0 then
      ball.vy = -3.4 - math.random() * 1.2
      ball.vx = (ball.x - 64) * 0.06 + (math.random() - 0.5) * 1.5
      tone(params:get("root") - 12, 0.14, 0.4, 0)
    end
  end
  if ball.y > 62 then launch() end
end

function init()
  local nn = function(p) return MusicUtil.note_num_to_name(p:get(), true) end
  params:add_separator("PINBALL")
  params:add_option("chord", "chord", CHORDS, 1)
  params:set_action("chord", build)
  params:add_number("root", "root", 48, 72, 60, nn)
  params:set_action("root", build)
  params:add_control("skill", "flipper skill", controlspec.new(0, 1, 'lin', 0.01, 0.8, ''))
  params:add_control("gravity", "gravity", controlspec.new(0.03, 0.2, 'lin', 0, 0.08, ''))
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 2600, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  math.randomseed(os.time())
  build()
  launch()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then params:set("root", util.clamp(msg.note, 48, 72)) end
  end
  metro.init(function()
    if not frozen then physics() end
    flip = math.max(0, flip - 1)
    for _, b in ipairs(bumpers) do b.lit = math.max(0, (b.lit or 0) - 1) end
    redraw()
  end, 1 / 40):start()
end

function enc(n, d)
  if n == 2 then params:delta("chord", d)
  elseif n == 3 then params:delta("skill", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then flip = 8 elseif n == 3 then frozen = not frozen end
end

function redraw()
  screen.clear()
  screen.level(3)
  screen.rect(3, 10, 122, 49)
  screen.stroke()
  for _, b in ipairs(bumpers) do
    local lit = (b.lit or 0) > 0
    screen.level(lit and 15 or 5)
    screen.circle(b.x, b.y, b.r)
    if lit then screen.fill() else screen.stroke() end
  end
  local lift = flip > 0 and -5 or 0
  screen.level(flip > 0 and 15 or 8)
  screen.move(36, 53) screen.line(58, 57 + lift) screen.stroke()
  screen.move(92, 53) screen.line(70, 57 + lift) screen.stroke()
  screen.level(15)
  screen.circle(ball.x, ball.y, 1.5)
  screen.fill()
  screen.move(0, 7)
  screen.text(frozen and "pinball (tilt)" or "pinball")
  screen.move(127, 7)
  screen.text_right(score)
  screen.level(4)
  screen.move(0, 62)
  screen.text(MusicUtil.note_num_to_name(params:get("root"), false) .. " " .. CHORDS[params:get("chord")])
  screen.move(127, 62)
  screen.text_right("skill " .. params:string("skill"))
  screen.update()
end
