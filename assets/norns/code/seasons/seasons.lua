-- seasons
-- a Portamax norns script
--
-- one year turns on the wheel.
-- spring is lydian and quickening,
-- summer major and bright, autumn
-- dorian and slowing, winter minor,
-- sparse and low. mode, tempo and
-- note density follow the season.
--
-- E2 year length   E3 brightness
-- K2 skip a season   K3 pause
-- pads: play in the season's mode
-- (params: root, year length)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local SEASONS = {
  { name = "spring", scale = "Lydian", bpm = 96, density = 0.7, oct = 12, rel = 0.9 },
  { name = "summer", scale = "Major", bpm = 120, density = 0.85, oct = 12, rel = 0.6 },
  { name = "autumn", scale = "Dorian", bpm = 84, density = 0.6, oct = 0, rel = 1.4 },
  { name = "winter", scale = "Natural Minor", bpm = 60, density = 0.4, oct = -12, rel = 2.6 },
}
local year = 0.02 -- 0..1, starts in early spring
local paused = false
local deg = 1
local last = {}

local function season()
  return SEASONS[math.floor(year * 4) % 4 + 1], (year * 4) % 1
end

local function play(n, rel)
  engine.release(rel)
  engine.hz(MusicUtil.note_num_to_freq(n))
  table.insert(last, 1, n)
  last[9] = nil
end

local function tick()
  local s, frac = season()
  local nxt = SEASONS[math.floor(year * 4 + 1) % 4 + 1]
  local scale = MusicUtil.generate_scale_of_length(params:get("root") + s.oct, s.scale, 15)
  if math.random() < s.density then
    -- a melody that walks, leaning toward the middle
    deg = util.clamp(deg + math.random(-2, 2) + (deg > 10 and -1 or (deg < 4 and 1 or 0)), 1, #scale)
    engine.pan((math.random() - 0.5) * 0.8)
    engine.pw(0.3 + 0.3 * math.sin(year * 2 * math.pi))
    play(scale[deg], s.rel + (nxt.rel - s.rel) * frac)
  end
  -- a low root at the start of each bar
  if math.random() < 0.25 then play(scale[1] - (scale[1] >= 48 and 12 or 0), 2.5) end
end

-- tempo glides between this season's and the next
local function interval()
  local s, frac = season()
  local nxt = SEASONS[math.floor(year * 4 + 1) % 4 + 1]
  return 60 / (s.bpm + (nxt.bpm - s.bpm) * frac) / 2
end

function init()
  params:add_separator("SEASONS")
  params:add_number("root", "root", 36, 72, 55, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:add_control("year", "year length", controlspec.new(20, 600, 'exp', 1, 120, 's'))
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 1800, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  engine.amp(0.24)
  math.randomseed(os.time())
  play(params:get("root"), 2) -- the year begins
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local s = season()
      local sc = MusicUtil.generate_scale_of_length(params:get("root") - 12, s.scale, 36)
      play(MusicUtil.snap_note_to_array(msg.note, sc), 1.5)
    end
  end
  clock.run(function()
    while true do
      if not paused then tick() end
      local dt = interval()
      clock.sleep(dt)
      if not paused then year = (year + dt / params:get("year")) % 1 end
    end
  end)
  metro.init(redraw, 1 / 15):start()
end

function enc(n, d)
  if n == 2 then params:delta("year", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then year = (math.floor(year * 4) + 1) / 4 % 1 + 0.01 tick()
  elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  local s = season()
  local cx, cy, r = 30, 34, 18
  -- the year wheel, a quarter per season
  for i = 0, 3 do
    screen.level(SEASONS[i + 1] == s and 10 or 2)
    screen.arc(cx, cy, r, i * math.pi / 2 - math.pi / 2 + 0.08, (i + 1) * math.pi / 2 - math.pi / 2 - 0.08)
    screen.stroke()
  end
  local a = year * 2 * math.pi - math.pi / 2
  screen.level(15)
  screen.move(cx, cy)
  screen.line(cx + math.cos(a) * (r - 3), cy + math.sin(a) * (r - 3))
  screen.stroke()
  -- the sun rides high in summer and low in winter
  local h = 24 - 10 * math.cos((year - 0.375) * 2 * math.pi)
  screen.level(s.name == "winter" and 5 or 12)
  screen.circle(100, h, 4)
  screen.fill()
  -- the last notes, as a little staff
  for i, n in ipairs(last) do
    screen.level(math.max(1, 12 - i))
    screen.rect(118 - i * 6, 54 - (n % 24) / 2, 3, 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "seasons (paused)" or "seasons")
  screen.move(127, 8)
  screen.text_right(s.name)
  screen.level(4)
  screen.move(0, 62)
  screen.text(string.lower(s.scale))
  screen.move(127, 62)
  screen.text_right(string.format("%d bpm", math.floor(60 / interval() / 2 + 0.5)))
  screen.update()
end
