-- lsystem
-- a Portamax norns script
--
-- a little grammar rewrites a
-- word over and over, the way
-- plants branch. a turtle reads
-- the word: F plays and steps,
-- + and - turn the melody up and
-- down the scale, [ ] remember and
-- return.
--
-- E2 generation   E3 rule
-- K2 restart   K3 hold
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local RULES = {
  { name = "fern", F = "F[+F]F[-F]F" },
  { name = "bush", F = "FF-[-F+F+F]+[+F-F-F]" },
  { name = "weed", F = "F[+F]F[-F][F]" },
  { name = "vine", F = "F+F--F+F" },
}
local word = "F"
local pos = 1
local degree = 8
local stack = {}
local scale = {}
local held = false
local trail = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 24)
end

local function grow()
  local rule = RULES[params:get("rule")].F
  word = "F"
  for _ = 1, params:get("generation") do
    local out = {}
    for c in word:gmatch(".") do out[#out + 1] = (c == "F") and rule or c end
    word = table.concat(out)
    if #word > 2000 then break end
  end
  pos, degree, stack = 1, 8, {}
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  local rule_names = {}
  for i, r in ipairs(RULES) do rule_names[i] = r.name end
  params:add_separator("LSYSTEM")
  params:add_number("generation", "generation", 1, 4, 2)
  params:set_action("generation", grow)
  params:add_option("rule", "rule", rule_names, 1)
  params:set_action("rule", grow)
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.release(0.6)
  engine.amp(0.25)
  build_scale()
  grow()
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      if not held then walk() end
      redraw()
    end
  end)
end

function walk()
  -- advance to the next F, applying turns and brackets on the way
  local guard = 0
  while guard < 64 do
    guard = guard + 1
    if pos > #word then pos, degree, stack = 1, 8, {} end
    local c = word:sub(pos, pos)
    pos = pos + 1
    if c == "+" then degree = degree + 1
    elseif c == "-" then degree = degree - 1
    elseif c == "[" then table.insert(stack, degree)
    elseif c == "]" then degree = table.remove(stack) or 8
    elseif c == "F" then
      degree = util.clamp(degree, 1, #scale)
      engine.cutoff(800 + #stack * 900)
      engine.pan((#stack % 3 - 1) * 0.5)
      engine.hz(MusicUtil.note_num_to_freq(scale[degree]))
      table.insert(trail, { d = degree, s = #stack })
      while #trail > 40 do table.remove(trail, 1) end
      return
    end
  end
end

function enc(n, d)
  if n == 2 then params:delta("generation", d)
  elseif n == 3 then params:delta("rule", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then grow() elseif n == 3 then held = not held end
  redraw()
end

function redraw()
  screen.clear()
  local x, y = 4, 40
  screen.level(2)
  for i, t in ipairs(trail) do
    local nx = 4 + i * 3
    local ny = 56 - t.d * 2
    screen.level(3 + math.min(12, t.s * 4))
    screen.move(x, y)
    screen.line(nx, ny)
    screen.stroke()
    x, y = nx, ny
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("lsystem: " .. RULES[params:get("rule")].name .. (held and " (held)" or ""))
  screen.level(4)
  screen.move(127, 62)
  screen.text_right("gen " .. params:get("generation") .. "  " .. pos .. "/" .. #word)
  screen.update()
end
