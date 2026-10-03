-- billiards
-- a Portamax norns script
--
-- a cue ball breaks a rack of
-- tuned balls. every knock rings
-- the struck ball's note, louder
-- and brighter the harder it hits.
-- sink a ball and its chord plays.
-- the table re-racks when cleared.
--
-- E2 shot power   E3 cloth (friction)
-- K2 shoot   K3 pause
-- pads: change key
-- (params: scale, root, brightness)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local L, R, T, B, RAD = 6, 122, 12, 53, 2.5
local POCKETS = { { L, T }, { 64, T - 1 }, { R, T }, { L, B }, { 64, B + 1 }, { R, B } }
local balls = {}
local scale = {}
local paused = false
local idle = 0
local sunk = 0
local rings = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function tone(n, amp, pan, rel, bright)
  engine.amp(amp)
  engine.pan(pan)
  engine.release(rel)
  engine.cutoff(params:get("bright") * bright)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function rack()
  balls = { { x = 30, y = 33, vx = 0, vy = 0, deg = 0 } }
  local d = 1
  for row = 0, 2 do
    for k = 0, row do
      balls[#balls + 1] = { x = 82 + row * 4.4, y = 33 + (k - row / 2) * 5.2, vx = 0, vy = 0, deg = d }
      d = d + 1
    end
  end
end

local function shoot()
  local cue = balls[1]
  local target = balls[math.random(2, math.max(2, #balls))]
  if not target then rack() target = balls[2] end
  local dx, dy = target.x - cue.x, target.y - cue.y + (math.random() - 0.5) * 3
  local d = math.max(1, math.sqrt(dx * dx + dy * dy))
  local p = params:get("power") * (0.8 + math.random() * 0.4)
  cue.vx, cue.vy = dx / d * p, dy / d * p
  idle = 0
  tone(scale[1] - 12, 0.15, (cue.x - 64) / 64, 0.3, 0.6)
end

local function physics()
  local f = 1 - params:get("cloth")
  local moving = false
  for i = #balls, 1, -1 do
    local b = balls[i]
    b.x, b.y = b.x + b.vx, b.y + b.vy
    b.vx, b.vy = b.vx * f, b.vy * f
    if math.abs(b.vx) + math.abs(b.vy) > 0.3 then moving = true end
    local hitx = b.x < L + RAD or b.x > R - RAD
    local hity = b.y < T + RAD or b.y > B - RAD
    if hitx then b.vx = -b.vx b.x = util.clamp(b.x, L + RAD, R - RAD) end
    if hity then b.vy = -b.vy b.y = util.clamp(b.y, T + RAD, B - RAD) end
    if (hitx or hity) and math.abs(b.vx) + math.abs(b.vy) > 1 then
      -- the cushion answers with a soft low knock
      tone(scale[1] - 12 + (hitx and 7 or 0), 0.08, (b.x - 64) / 64, 0.3, 0.5)
    end
    for _, p in ipairs(POCKETS) do
      if (b.x - p[1]) ^ 2 + (b.y - p[2]) ^ 2 < 30 then
        rings[#rings + 1] = { x = p[1], y = p[2], t = 12 }
        if i == 1 then
          -- scratch: a low groan, the cue comes back
          tone(scale[1] - 24, 0.3, 0, 1.5, 0.4)
          b.x, b.y, b.vx, b.vy = 30, 33, 0, 0
        else
          sunk = sunk + 1
          local c = { scale[b.deg], scale[b.deg + 2], scale[b.deg + 4] }
          for _, n in ipairs(c) do tone(n, 0.2, (p[1] - 64) / 64, 2.4, 1) end
          table.remove(balls, i)
        end
        break
      end
    end
  end
  for i = 1, #balls do
    for j = i + 1, #balls do
      local a, b = balls[i], balls[j]
      local dx, dy = b.x - a.x, b.y - a.y
      local d = math.sqrt(dx * dx + dy * dy)
      if d < RAD * 2 and d > 0 then
        local nx, ny = dx / d, dy / d
        local rel = (a.vx - b.vx) * nx + (a.vy - b.vy) * ny
        if rel > 0 then
          a.vx, a.vy = a.vx - rel * nx, a.vy - rel * ny
          b.vx, b.vy = b.vx + rel * nx, b.vy + rel * ny
          local n = scale[math.max(a.deg, b.deg) + 1]
          tone(n, util.clamp(0.06 + rel * 0.07, 0.06, 0.32), (a.x - 64) / 64, 0.4 + rel * 0.3, util.clamp(0.4 + rel * 0.3, 0.4, 2))
        end
        local push = (RAD * 2 - d) / 2
        a.x, a.y, b.x, b.y = a.x - nx * push, a.y - ny * push, b.x + nx * push, b.y + ny * push
      end
    end
  end
  if #balls == 1 then rack() end
  idle = idle + 1
  -- the next shot comes once the table settles, or after a few seconds
  if (not moving and idle > 20) or idle > 110 then shoot() end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("BILLIARDS")
  params:add_option("scale", "scale", names, 4)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 48, 72, 57, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("power", "shot power", controlspec.new(1, 8, 'lin', 0, 4.5, ''))
  params:add_control("cloth", "cloth", controlspec.new(0.005, 0.06, 'exp', 0, 0.018, ''))
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 1800, 'hz'))
  params:default()
  math.randomseed(os.time())
  build_scale()
  rack()
  shoot()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then params:set("root", msg.note - 12 * (msg.note > 66 and 1 or 0)) end
  end
  metro.init(function()
    if not paused then physics() end
    for i = #rings, 1, -1 do
      rings[i].t = rings[i].t - 1
      if rings[i].t <= 0 then table.remove(rings, i) end
    end
    redraw()
  end, 1 / 30):start()
end

function enc(n, d)
  if n == 2 then params:delta("power", d)
  elseif n == 3 then params:delta("cloth", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then shoot() elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  screen.level(3)
  screen.rect(L - 1, T - 1, R - L + 2, B - T + 2)
  screen.stroke()
  for _, p in ipairs(POCKETS) do
    screen.level(1)
    screen.circle(p[1], p[2], 3)
    screen.fill()
  end
  for _, r in ipairs(rings) do
    screen.level(r.t)
    screen.circle(r.x, r.y, 14 - r.t)
    screen.stroke()
  end
  for i, b in ipairs(balls) do
    screen.level(i == 1 and 15 or 4 + b.deg)
    screen.circle(b.x, b.y, RAD)
    if i == 1 or b.deg % 2 == 0 then screen.fill() else screen.stroke() end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("billiards")
  screen.move(127, 8)
  screen.text_right("sunk " .. sunk)
  screen.level(4)
  screen.move(0, 62)
  screen.text(paused and "paused" or MusicUtil.note_num_to_name(params:get("root"), false) .. " " .. string.lower(MusicUtil.SCALES[params:get("scale")].name))
  screen.move(127, 62)
  screen.text_right("power " .. params:string("power"))
  screen.update()
end
