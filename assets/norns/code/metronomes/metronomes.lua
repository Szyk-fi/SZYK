-- metronomes
-- a Portamax norns script
--
-- five metronomes, each set to a
-- slightly different tempo, stand
-- on one loose platform. its sway
-- couples them (kuramoto-style)
-- and they slowly fall into step.
-- every tick clicks that
-- metronome's pitch.
--
-- E2 coupling   E3 tempo spread
-- K2 scatter    K3 pin the platform
-- pads: nudge one metronome
-- (params: scale, root, tempo)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local N = 5
local mets = {}
local scale = {}
local pinned = false
local sway = 0
local order = 0
local t = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 14)
end

local function set_rates()
  local base = params:get("bpm") / 60 * math.pi -- two ticks per cycle
  local spread = params:get("spread")
  for i, m in ipairs(mets) do
    m.w = base * (1 + spread * m.offset)
  end
end

local function scatter()
  for i = 1, N do
    mets[i] = mets[i] or {}
    local m = mets[i]
    m.th = math.random() * 2 * math.pi
    m.offset = m.offset or ((i - (N + 1) / 2) / N + (math.random() - 0.5) * 0.15)
    m.deg = ({ 1, 3, 5, 8, 10 })[i]
    m.flash = 0
  end
  set_rates()
end

local function click(i, side)
  local m = mets[i]
  -- a click: a narrow pulse with a very short release; the right swing
  -- sounds a fifth above the left
  local deg = m.deg + (side > 0 and 4 or 0)
  engine.pan((i - (N + 1) / 2) / N * 1.6)
  engine.amp(0.2)
  engine.pw(0.08)
  engine.release(0.09 + params:get("ring"))
  engine.cutoff(params:get("bright"))
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(deg, 1, #scale)]))
  m.flash = 15
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("METRONOMES")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 48, 76, 62, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("bpm", "tempo", 40, 180, 92)
  params:set_action("bpm", function() if #mets > 0 then set_rates() end end)
  params:add_control("coupling", "coupling", controlspec.new(0, 3, 'lin', 0, 0.6, ''))
  params:add_control("spread", "tempo spread", controlspec.new(0, 0.3, 'lin', 0, 0.08, ''))
  params:set_action("spread", function() if #mets > 0 then set_rates() end end)
  params:add_control("ring", "ring", controlspec.new(0, 1, 'lin', 0, 0.15, 's'))
  params:add_control("bright", "brightness", controlspec.new(500, 9000, 'exp', 0, 4000, 'hz'))
  math.randomseed(os.time())
  scatter()
  params:default()
  engine.gain(1.4)
  build_scale()
  -- the first metronome is just about to tick
  mets[1].th = math.pi / 2 - 0.05
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local i = (msg.note % N) + 1
      mets[i].th = mets[i].th + 1.2
      click(i, 1)
    end
  end
  local m = metro.init(step, 1 / 60)
  m:start()
end

function step()
  local dt = 1 / 60
  t = t + dt
  local K = pinned and 0 or params:get("coupling")
  -- mean field: the platform responds to the sum of all the swings
  local sx, sy = 0, 0
  for _, m in ipairs(mets) do sx = sx + math.cos(m.th) sy = sy + math.sin(m.th) end
  order = math.sqrt(sx * sx + sy * sy) / N
  local psi = math.atan(sy, sx)
  for i, m in ipairs(mets) do
    -- keep theta small; 200*pi keeps the tick parity intact
    if m.th > 1000 then m.th = m.th - 200 * math.pi end
    local before = m.th
    m.th = m.th + (m.w + K * order * math.sin(psi - m.th)) * dt
    -- ticks at each extreme of the swing: theta = pi/2 + k*pi
    local a = math.floor((before - math.pi / 2) / math.pi)
    local b = math.floor((m.th - math.pi / 2) / math.pi)
    if b ~= a then click(i, (b % 2 == 0) and 1 or -1) end
    m.flash = math.max(0, m.flash - 1)
  end
  -- the platform swings against the pendulums' combined pull
  local pull = 0
  for _, m in ipairs(mets) do pull = pull + math.sin(m.th) end
  sway = pinned and 0 or (-pull / N * 3)
  if math.floor(t * 60 + 0.5) % 2 == 0 then redraw() end
end

function enc(n, d)
  if n == 2 then params:delta("coupling", d)
  elseif n == 3 then params:delta("spread", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then scatter()
  elseif n == 3 then pinned = not pinned end
end

function redraw()
  screen.clear()
  screen.line_width(1)
  local px = sway
  -- platform on two rollers
  screen.level(8)
  screen.rect(6 + px, 54, 116, 3)
  screen.fill()
  screen.level(pinned and 12 or 4)
  screen.circle(24, 60, 2.5)
  screen.circle(104, 60, 2.5)
  if pinned then screen.fill() else screen.stroke() end
  for i, m in ipairs(mets) do
    local bx = 18 + (i - 1) * 23 + px
    -- body: a little pyramid
    screen.level(5)
    screen.move(bx - 7, 54)
    screen.line(bx - 3, 24)
    screen.line(bx + 3, 24)
    screen.line(bx + 7, 54)
    screen.close()
    screen.stroke()
    local ang = math.sin(m.th) * 0.55
    local tipx = bx + math.sin(ang) * 26
    local tipy = 50 - math.cos(ang) * 26
    screen.level(math.max(9, m.flash))
    screen.move(bx, 50)
    screen.line(tipx, tipy)
    screen.stroke()
    screen.rect(tipx - 1.5, tipy + 4, 3, 3)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(pinned and "metronomes (pinned)" or "metronomes")
  -- order parameter: how in step they are
  screen.level(3)
  screen.rect(88, 3, 38, 5)
  screen.stroke()
  screen.level(12)
  screen.rect(89, 4, math.floor(36 * order), 3)
  screen.fill()
  screen.update()
end
