-- radio
-- a Portamax norns script
--
-- an old radio scanning the dial.
-- between stations there is only
-- crackle; as the needle nears a
-- station its tune fades up through
-- the static. each station has its
-- own song, key and tempo.
--
-- E2 tune by hand   E3 static
-- K2 seek next station   K3 scan on / off
-- pads: thump the cabinet
-- (params: tone, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local STATIONS = {
  { f = 89.4, name = "KLMN", root = 57, scale = "Dorian", div = 1 / 2 },
  { f = 93.1, name = "WAVE", root = 62, scale = "Major Pentatonic", div = 1 / 4 },
  { f = 97.7, name = "NITE", root = 52, scale = "Natural Minor", div = 1 / 2 },
  { f = 101.9, name = "SURF", root = 60, scale = "Mixolydian", div = 1 / 4 },
  { f = 106.3, name = "OWL", root = 55, scale = "Minor Pentatonic", div = 1 },
}
local dial = 89.0
local target = 1
local scanning = true
local linger = 0
local static_lvl = 0
local hand = 0

local function make_song(s)
  local sc = MusicUtil.generate_scale_of_length(s.root, s.scale, 10)
  s.song, s.pos = {}, 0
  for i = 1, 16 do s.song[i] = (math.random() < 0.2) and 0 or sc[math.random(#sc)] end
  s.song[1] = sc[1]
end

local function nearest()
  local best, bd = 1, 99
  for i, s in ipairs(STATIONS) do
    local d = math.abs(s.f - dial)
    if d < bd then best, bd = i, d end
  end
  return best, util.clamp(1 - bd / 1.2, 0, 1)
end

function init()
  params:add_separator("RADIO")
  params:add_control("static", "static", controlspec.new(0, 1, 'lin', 0, 0.6, ''))
  params:add_control("tone", "tone", controlspec.new(500, 5000, 'exp', 0, 1800, 'hz'))
  params:add_control("release", "release", controlspec.new(0.1, 2, 'lin', 0, 0.5, 's'))
  params:default()
  math.randomseed(os.time())
  for _, s in ipairs(STATIONS) do make_song(s) end
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      dial = util.clamp(dial + (math.random() - 0.5) * 0.8, 88, 108)
      engine.release(0.4) engine.amp(0.3) engine.cutoff(400) engine.pw(0.1)
      engine.hz(MusicUtil.note_num_to_freq(msg.note - 24))
      static_lvl = 10
    end
  end
  -- each station keeps playing whether you hear it or not
  for i, s in ipairs(STATIONS) do
    clock.run(function()
      while true do
        clock.sync(s.div)
        s.pos = s.pos % #s.song + 1
        local n = s.song[s.pos]
        local near, strength = nearest()
        if near == i and n > 0 and strength > 0.02 then
          engine.release(params:get("release")) engine.amp(0.3 * strength)
          engine.cutoff(params:get("tone") * (0.4 + 0.6 * strength)) engine.pw(0.4) engine.pan(0)
          engine.hz(MusicUtil.note_num_to_freq(n))
        end
      end
    end)
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      update()
      redraw()
    end
  end)
end

function update()
  local near, strength = nearest()
  hand = math.max(0, hand - 1)
  if scanning and hand == 0 then
    local t = STATIONS[target].f
    if math.abs(t - dial) < 0.05 then
      linger = linger + 1
      if linger > 200 then target = target % #STATIONS + 1 linger = 0 end
    else
      dial = dial + util.clamp(t - dial, -0.06, 0.06)
    end
  end
  -- crackle: little bursts of random high clicks between stations
  static_lvl = math.max(0, static_lvl - 1)
  local amount = (1 - strength) * params:get("static")
  if math.random() < amount * 0.5 then
    for _ = 1, math.random(1, 3) do
      engine.release(0.02 + math.random() * 0.04) engine.amp(0.05 + 0.08 * amount)
      engine.cutoff(6000) engine.pw(math.random()) engine.pan(math.random() - 0.5)
      engine.hz(1500 + math.random() * 5000)
    end
    static_lvl = math.floor(amount * 8)
  end
end

function enc(n, d)
  if n == 2 then dial = util.clamp(dial + d * 0.1, 88, 108) hand = 90
  elseif n == 3 then params:delta("static", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then target = nearest() % #STATIONS + 1 linger = 0 hand = 0
  elseif n == 3 then scanning = not scanning end
  redraw()
end

function redraw()
  screen.clear()
  local function x_of(f) return util.linlin(88, 108, 6, 122, f) end
  screen.level(3)
  screen.rect(2, 14, 124, 26)
  screen.stroke()
  for f = 88, 108, 2 do
    screen.level(f % 4 == 0 and 6 or 3)
    screen.move(x_of(f), 30) screen.line(x_of(f), f % 4 == 0 and 25 or 27) screen.stroke()
  end
  for _, s in ipairs(STATIONS) do
    screen.level(5)
    screen.rect(x_of(s.f) - 1, 33, 2, 2) screen.fill()
  end
  screen.level(15)
  screen.move(x_of(dial), 16) screen.line(x_of(dial), 38) screen.stroke()
  local near, strength = nearest()
  for i = 0, 9 do
    screen.level(i < strength * 10 and 12 or 2)
    screen.rect(4 + i * 6, 46, 4, 5) screen.fill()
  end
  for _ = 1, static_lvl * 3 do
    screen.level(math.random(1, 6))
    screen.pixel(math.random(66, 125), math.random(44, 52)) screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(scanning and "radio (scan)" or "radio")
  screen.move(127, 8)
  screen.text_right(strength > 0.6 and STATIONS[near].name or "")
  screen.level(4)
  screen.move(0, 62)
  screen.text(string.format("%.1f MHz", dial))
  screen.move(127, 62)
  screen.text_right("static " .. params:string("static"))
  screen.update()
end
