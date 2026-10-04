-- constellations
-- a Portamax norns script
--
-- stars appear one by one, each with
-- a faint high note. then they are
-- joined into a constellation, line
-- by line: each line plays the star
-- it reaches, higher in the sky is
-- higher in pitch. then it is named.
--
-- E2 stars per figure   E3 pace
-- K2 clear the sky   K3 pause
-- pads: place a star
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local SYL = { "al", "ver", "is", "or", "ka", "lyn", "tes", "mir", "on", "dra", "sel", "ux" }
local PACE = { 1 / 4, 1 / 2, 1, 2 }
local scale = {}
local cur = { stars = {}, lines = {}, name = nil }
local old = {}
local phase = "stars"
local hold = 0
local paused = false
local glow = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function star_note(s)
  return scale[util.clamp(util.round(util.linlin(54, 12, 1, #scale, s.y)), 1, #scale)]
end

local function play(n, amp, x)
  engine.amp(amp)
  engine.pan(util.linlin(0, 128, -0.7, 0.7, x or 64))
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function add_star(x, y)
  local s = { x = x or math.random(8, 120), y = y or math.random(14, 52), linked = false }
  table.insert(cur.stars, s)
  play(star_note(s) + 12, 0.1, s.x)
  return s
end

local function make_name()
  local n = SYL[math.random(#SYL)] .. SYL[math.random(#SYL)]
  if math.random() < 0.5 then n = n .. SYL[math.random(#SYL)] end
  return n:sub(1, 1):upper() .. n:sub(2)
end

local function link()
  -- join the last linked star to the nearest unlinked one
  local from = cur.lines[#cur.lines] and cur.lines[#cur.lines][2] or cur.stars[1]
  from.linked = true
  local best, bd = nil, 1e9
  for _, s in ipairs(cur.stars) do
    if not s.linked then
      local d = (s.x - from.x) ^ 2 + (s.y - from.y) ^ 2
      if d < bd then best, bd = s, d end
    end
  end
  if not best then return false end
  best.linked = true
  table.insert(cur.lines, { from, best })
  play(star_note(best), util.linlin(0, 4000, 0.3, 0.16, bd), best.x)
  glow = 4
  return true
end

function tick()
  if phase == "stars" then
    add_star()
    if #cur.stars >= params:get("count") then phase = "lines" end
  elseif phase == "lines" then
    if not link() then phase = "hold" hold = 0 cur.name = make_name() end
  elseif phase == "hold" then
    hold = hold + 1
    local s = cur.stars[(hold - 1) % #cur.stars + 1]
    play(star_note(s) - 12, 0.12, s.x)
    if hold >= #cur.stars then
      table.insert(old, 1, cur)
      if #old > 2 then table.remove(old) end
      cur = { stars = {}, lines = {}, name = nil }
      phase = "stars"
    end
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("CONSTELLATIONS")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 48, 72, 57, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("count", "stars per figure", 3, 9, 6)
  params:add_option("pace", "pace (beats)", { "1/4", "1/2", "1", "2" }, 2)
  params:add_control("release", "release", controlspec.new(0.3, 5, 'exp', 0, 2.2, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(2200)
  engine.pw(0.45)
  math.randomseed(os.time())
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      add_star(math.random(8, 120), util.clamp(54 - (msg.note - 60) * 3, 14, 52))
      if phase ~= "stars" and phase ~= "lines" then phase = "lines" end
    end
  end
  tick()
  clock.run(function()
    while true do
      clock.sync(PACE[params:get("pace")])
      if not paused then tick() end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 15)
      glow = math.max(0, glow - 1)
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("count", d)
  elseif n == 3 then params:delta("pace", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then old = {} cur = { stars = {}, lines = {} } phase = "stars" add_star()
  elseif n == 3 then paused = not paused end
  redraw()
end

local function draw_fig(f, lvl_star, lvl_line)
  screen.level(lvl_line)
  for _, l in ipairs(f.lines) do
    screen.move(l[1].x, l[1].y) screen.line(l[2].x, l[2].y) screen.stroke()
  end
  screen.level(lvl_star)
  for _, s in ipairs(f.stars) do screen.rect(s.x - 1, s.y - 1, 2, 2) screen.fill() end
end

function redraw()
  screen.clear()
  for i, f in ipairs(old) do draw_fig(f, 3 - i, 1) end
  draw_fig(cur, 15, glow > 0 and 12 or 6)
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "constellations (paused)" or "constellations")
  screen.level(4)
  screen.move(0, 62)
  screen.text(params:get("count") .. " stars")
  screen.move(127, 62)
  screen.text_right(cur.name or (old[1] and old[1].name) or "")
  screen.update()
end
