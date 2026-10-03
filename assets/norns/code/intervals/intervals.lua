-- intervals
-- a Portamax norns script
--
-- ear training. an interval plays
-- over and over: two notes apart,
-- then together. turn E3 to your
-- guess and press K3 to check. a
-- right answer moves on; a wrong
-- one shows the answer.
--
-- E2 difficulty   E3 guess
-- K2 skip   K3 check
-- (params: direction, tone)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local NAMES = { "minor 2nd", "major 2nd", "minor 3rd", "major 3rd", "perfect 4th", "tritone",
  "perfect 5th", "minor 6th", "major 6th", "minor 7th", "major 7th", "octave" }
-- each level adds intervals, easiest (most distinct) first
local POOLS = { { 12, 7, 4 }, { 12, 7, 5, 4, 3 }, { 12, 7, 5, 4, 3, 9, 2 }, { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12 } }
local answer = 7
local low = 60
local guess = 1
local score, tries = 0, 0
local verdict = ""
local verdict_t = 0
local playing = 0

local function new_question()
  local pool = POOLS[params:get("level")]
  local a = pool[math.random(#pool)]
  if a == answer and #pool > 1 then a = pool[math.random(#pool)] end
  answer = a
  low = math.random(55, 67)
end

local function sound(n, amp)
  engine.amp(amp)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

function init()
  params:add_separator("INTERVALS")
  params:add_number("level", "difficulty", 1, 4, 1)
  params:add_option("dir", "direction", { "up", "down", "either" }, 1)
  params:add_control("tone", "tone", controlspec.new(400, 4000, 'exp', 0, 1500, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:default()
  engine.release(0.9)
  engine.pw(0.5)
  math.randomseed(os.time())
  new_question()
  clock.run(function()
    while true do
      -- melodic: first, second; then harmonic: both together
      local d = params:get("dir")
      local down = d == 2 or (d == 3 and low % 2 == 0)
      local a, b = low, low + answer
      if down then a, b = b, a end
      playing = 1 sound(a, 0.28) clock.sync(1)
      playing = 2 sound(b, 0.28) clock.sync(1)
      playing = 3 sound(a, 0.2) sound(b, 0.2) clock.sync(2)
      playing = 0
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 10)
      verdict_t = math.max(0, verdict_t - 1)
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("level", d)
  elseif n == 3 then guess = util.clamp(guess + d, 1, 12) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    verdict, verdict_t = "it was " .. NAMES[answer], 20
    new_question()
  elseif n == 3 then
    tries = tries + 1
    if guess == answer then
      score = score + 1
      verdict, verdict_t = "yes! " .. NAMES[answer], 20
      new_question()
    else
      verdict, verdict_t = "no - " .. NAMES[answer], 30
    end
  end
  redraw()
end

function redraw()
  screen.clear()
  -- the guess dial: twelve slots, the chosen one bright
  for i = 1, 12 do
    local x = 4 + (i - 1) * 10
    screen.level(i == guess and 15 or 3)
    screen.rect(x, 44 - i * 1.4, 8, i * 1.4)
    screen.fill()
  end
  for k = 1, 3 do
    screen.level(playing == k and 15 or 2)
    screen.circle(98 + k * 8, 5, 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("intervals")
  screen.level(10)
  screen.move(0, 53)
  screen.text("guess: " .. NAMES[guess])
  screen.level(verdict_t > 0 and 15 or 0)
  screen.move(0, 20)
  if verdict_t > 0 then screen.text(verdict) end
  screen.level(4)
  screen.move(0, 62)
  screen.text("level " .. params:get("level"))
  screen.move(127, 62)
  screen.text_right(score .. "/" .. tries)
  screen.update()
end
