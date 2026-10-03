-- strum
-- a Portamax norns script
--
-- a six-string strummer. each beat
-- the chord is swept down or up,
-- string by string. it walks a
-- progression on its own; press a
-- pad and that note becomes the
-- chord root (pads 1-8, C major).
--
-- E2 strum speed   E3 pattern
-- K2 chord quality   K3 next chord
-- (params: brightness, ring)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local PATTERNS = {
  { name = "D D U U D U", s = { "d", "d", "u", "u", "d", "u" } },
  { name = "D - D U - U", s = { "d", "", "d", "u", "", "u" } },
  { name = "D U D U", s = { "d", "u", "d", "u" } },
  { name = "D - - D - -", s = { "d", "", "", "d", "", "" } },
}
-- diatonic qualities of C major degrees by pitch class
local QUAL = { [0] = "major", [2] = "minor", [4] = "minor", [5] = "major", [7] = "dominant 7", [9] = "minor", [11] = "diminished" }
local PROG = { 0, 9, 5, 7 }
local prog_i = 1
local root = 48
local quality = "major"
local strings = {}
local lit = {}
local beat = 0
local dir = ""
local manual = false

local function voice_chord()
  local c = MusicUtil.generate_chord(root, quality, 0)
  strings = {}
  -- spread over six strings: low root, then chord tones climbing
  strings[1] = root - 12
  for i = 2, 6 do
    local t = c[(i - 2) % #c + 1] + 12 * ((i - 2) // #c)
    strings[i] = t
  end
end

local function set_root(n, q)
  root = n
  quality = q or QUAL[n % 12] or "major"
  voice_chord()
end

local function strum(direction)
  local gap = params:get("speed")
  local order = {}
  for i = 1, 6 do order[i] = direction == "d" and i or 7 - i end
  clock.run(function()
    for k, i in ipairs(order) do
      engine.pan((i - 3.5) / 5)
      engine.amp(direction == "d" and 0.2 or 0.15)
      engine.hz(MusicUtil.note_num_to_freq(strings[i]))
      lit[i] = 15
      if k < 6 then clock.sleep(gap) end
    end
  end)
end

function init()
  local names = {}
  for i, p in ipairs(PATTERNS) do names[i] = p.name end
  params:add_separator("STRUM")
  params:add_control("speed", "strum speed", controlspec.new(0.005, 0.08, 'exp', 0, 0.022, 's'))
  params:add_option("pattern", "pattern", names, 1)
  params:add_control("bright", "brightness", controlspec.new(400, 5000, 'exp', 0, 1800, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:add_control("ring", "ring", controlspec.new(0.2, 3, 'lin', 0, 1.2, 's'))
  params:set_action("ring", function(x) engine.release(x) end)
  params:default()
  engine.pw(0.35)
  set_root(48 + PROG[1])
  for i = 1, 6 do lit[i] = 0 end
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      manual = true
      set_root(48 + (msg.note - 60) % 12 + (msg.note >= 72 and 12 or 0))
    end
  end
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      local pat = PATTERNS[params:get("pattern")].s
      beat = beat % #pat + 1
      if beat == 1 and not manual then
        prog_i = prog_i % #PROG + 1
        set_root(48 + PROG[prog_i])
      end
      dir = pat[beat]
      if dir ~= "" then strum(dir) end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 20)
      for i = 1, 6 do lit[i] = math.max(0, lit[i] - 1) end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("pattern", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    local qs = { "major", "minor", "sus4", "major 7", "minor 7", "sus2" }
    local i = (tab.key(qs, quality) or 0) % #qs + 1
    set_root(root, qs[i])
  elseif n == 3 then
    manual = false
    prog_i = prog_i % #PROG + 1
    set_root(48 + PROG[prog_i])
  end
end

function redraw()
  screen.clear()
  for i = 1, 6 do
    local y = 14 + i * 6
    screen.level(math.max(2, lit[i]))
    screen.move(10, y)
    local wob = lit[i] > 0 and math.sin(lit[i]) * 1.5 or 0
    screen.curve(50, y + wob, 80, y - wob, 118, y)
    screen.stroke()
  end
  screen.level(dir == "" and 2 or 12)
  screen.move(4, 30)
  screen.text(dir == "d" and "v" or (dir == "u" and "^" or "-"))
  screen.level(15)
  screen.move(0, 8)
  screen.text("strum")
  screen.move(127, 8)
  screen.text_right(MusicUtil.note_num_to_name(root) .. " " .. quality)
  screen.level(4)
  screen.move(0, 62)
  screen.text(PATTERNS[params:get("pattern")].name)
  screen.move(127, 62)
  screen.text_right(manual and "pad" or "auto")
  screen.update()
end
