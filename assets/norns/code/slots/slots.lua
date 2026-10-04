-- slots
-- a Portamax norns script
--
-- a one-armed bandit that pays out
-- in harmony. the reels click to a
-- stop one by one; each lands on a
-- note and the three ring as a
-- chord. three of a kind: jackpot,
-- and the chord runs as an arpeggio.
--
-- E2 luck   E3 brightness
-- K2 pull the lever   K3 auto play
-- pads: pull the lever
-- (params: scale, root, hold)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local N = 6
local reels = {}
local state = "idle"
local timer = 0
local auto = true
local scale = {}
local jackpots, pulls = 0, 0
local glow = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 20)
end

local function tone(n, amp, rel, pan)
  engine.amp(amp)
  engine.release(rel)
  engine.pan(pan)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function reel_note(i)
  -- each reel stacks two scale steps above the one before it, so three
  -- matching symbols make a triad on that symbol's degree
  return scale[reels[i].sym + 2 * (i - 1)]
end

local function pull()
  pulls = pulls + 1
  -- luck decides ahead of time whether the reels will match
  local first = math.random(N)
  for i = 1, 3 do
    local target = (i > 1 and math.random() < params:get("luck")) and reels[i - 1].target or math.random(N)
    reels[i] = { pos = reels[i] and reels[i].pos or math.random(N), speed = 0.35 + i * 0.05, stopping = false,
      target = i == 1 and first or target, sym = nil, last = 0 }
  end
  state, timer = "spin", 0
  tone(params:get("root") - 12, 0.22, 0.8, 0)
end

local function land(i)
  local r = reels[i]
  r.sym = r.target
  r.pos = r.target
  tone(reel_note(i), 0.26, 2.2, (i - 2) * 0.5)
  if i == 3 then
    state, timer = "show", 0
    if reels[1].sym == reels[2].sym and reels[2].sym == reels[3].sym then
      jackpots = jackpots + 1
      glow = 40
      clock.run(function()
        for k = 0, 6 do
          clock.sleep(0.09)
          tone(scale[reels[1].sym + 2 * (k % 3)] + 12 * (k // 3), 0.2, 0.9, (k - 3) / 4)
        end
      end)
    end
  end
end

local function step()
  timer = timer + 1
  glow = math.max(0, glow - 1)
  if state == "spin" then
    for i, r in ipairs(reels) do
      if not r.sym then
        if timer == 12 + i * 12 then r.stopping = true end
        if r.stopping then
          -- ease the reel down so it settles on the target symbol
          local dist = (r.target - r.pos) % N
          if dist < 0.02 or (r.speed < 0.06 and dist < 0.3) then land(i)
          else r.speed = math.max(0.04, math.min(r.speed, dist * 0.12)) end
        end
        if not r.sym then
          r.pos = (r.pos + r.speed - 1) % N + 1
          local slot = math.floor(r.pos)
          if r.stopping and slot ~= r.last then tone(scale[10] + 12, 0.05, 0.08, (i - 2) * 0.5) end
          r.last = slot
        end
      end
    end
  elseif state == "show" and auto and timer > params:get("hold") * 30 then
    pull()
  end
  redraw()
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("SLOTS")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 48, 72, 55, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("luck", "luck", controlspec.new(0, 1, 'lin', 0.01, 0.35, ''))
  params:add_control("hold", "hold", controlspec.new(0.3, 4, 'lin', 0, 1.2, 's'))
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 2400, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  math.randomseed(os.time())
  build_scale()
  pull()
  local m = midi.connect()
  m.event = function(data)
    if midi.to_msg(data).type == "note_on" and state ~= "spin" then pull() end
  end
  metro.init(step, 1 / 30):start()
end

function enc(n, d)
  if n == 2 then params:delta("luck", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 and state ~= "spin" then pull()
  elseif n == 3 then auto = not auto end
end

local function symbol(s, x, y)
  if s == 1 then screen.circle(x, y, 4) screen.fill()
  elseif s == 2 then screen.rect(x - 4, y - 4, 8, 8) screen.fill()
  elseif s == 3 then screen.move(x, y - 5) screen.line(x + 5, y + 4) screen.line(x - 5, y + 4) screen.close() screen.fill()
  elseif s == 4 then screen.move(x, y - 5) screen.line(x + 5, y) screen.line(x, y + 5) screen.line(x - 5, y) screen.close() screen.fill()
  elseif s == 5 then for k = -3, 3, 3 do screen.rect(x - 5, y + k - 1, 10, 2) end screen.fill()
  else screen.move(x - 4, y - 4) screen.line(x + 4, y - 4) screen.line(x - 1, y + 5) screen.stroke() end
end

function redraw()
  screen.clear()
  for i, r in ipairs(reels) do
    local x = 30 + (i - 1) * 34
    screen.level(glow > 0 and (glow % 8 < 4 and 15 or 6) or 4)
    screen.rect(x - 13, 13, 26, 38)
    screen.stroke()
    local base = math.floor(r.pos)
    local frac = r.pos - base
    for k = -1, 1 do
      local s = (base + k - 1) % N + 1
      local y = 32 + (k - frac) * 13
      if y > 20 and y < 44 then
        screen.level(r.sym and k == 0 and 15 or (math.abs(y - 32) < 5 and 10 or 3))
        symbol(s, x, y)
      end
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(auto and "slots" or "slots (manual)")
  screen.move(127, 8)
  screen.text_right("jackpots " .. jackpots)
  screen.level(4)
  screen.move(0, 62)
  screen.text("pull " .. pulls)
  screen.move(127, 62)
  screen.text_right("luck " .. params:string("luck"))
  screen.update()
end
