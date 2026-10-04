-- acid
-- a Portamax norns script
--
-- a sixteen-step bassline. accents
-- open the filter and push harder;
-- slides stretch the release so a
-- note bleeds into the next one.
--
-- E2 cutoff   E3 accent depth
-- K2 mutate a few steps
-- K3 new random line
-- (params: root, scale, decay)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local seq = {}
local pos = 0
local scale = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale") == 1 and "Natural Minor" or "Phrygian", 10)
end

local function random_step()
  return {
    deg = math.random() < 0.45 and 1 or math.random(#scale),
    on = math.random() < 0.75,
    acc = math.random() < 0.3,
    slide = math.random() < 0.2,
    oct = math.random() < 0.15 and 12 or 0,
  }
end

local function randomise()
  for i = 1, 16 do seq[i] = random_step() end
  seq[1].on, seq[1].deg = true, 1
end

function init()
  params:add_separator("ACID")
  params:add_number("root", "root", 28, 48, 36, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_option("scale", "scale", { "minor", "phrygian" }, 1)
  params:set_action("scale", build_scale)
  params:add_control("cutoff", "cutoff", controlspec.new(150, 3000, 'exp', 0, 600, 'hz'))
  params:add_control("accent", "accent depth", controlspec.new(1, 5, 'lin', 0, 2.8, 'x'))
  params:add_control("decay", "decay", controlspec.new(0.05, 0.6, 'lin', 0, 0.16, 's'))
  params:default()
  build_scale()
  math.randomseed(os.time())
  randomise()
  engine.gain(2)
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      pos = pos % 16 + 1
      local s = seq[pos]
      if s.on then
        local acc = s.acc and params:get("accent") or 1
        engine.cutoff(util.clamp(params:get("cutoff") * acc, 80, 9000))
        engine.release(params:get("decay") * (s.slide and 3 or 1) * (s.acc and 1.3 or 1))
        engine.amp(s.acc and 0.32 or 0.2)
        engine.pw(s.slide and 0.3 or 0.15)
        engine.hz(MusicUtil.note_num_to_freq(scale[s.deg] + s.oct))
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("cutoff", d)
  elseif n == 3 then params:delta("accent", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    for _ = 1, 3 do seq[math.random(2, 16)] = random_step() end
  elseif n == 3 then randomise() end
  redraw()
end

function redraw()
  screen.clear()
  for i = 1, 16 do
    local s = seq[i]
    local x = 2 + (i - 1) * 7.8
    if s.on then
      local h = 4 + s.deg * 2 + (s.oct > 0 and 8 or 0)
      screen.level(i == pos and 15 or (s.acc and 9 or 4))
      screen.rect(x, 46 - h, 6, h)
      screen.fill()
      if s.slide then
        screen.level(6)
        screen.move(x + 6, 46 - h)
        screen.line(x + 8, 46 - h)
        screen.stroke()
      end
    end
    screen.level(i == pos and 15 or 2)
    screen.rect(x, 49, 6, 2)
    screen.fill()
    if s.acc then
      screen.level(10)
      screen.pixel(x + 2, 53)
      screen.fill()
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("acid")
  screen.level(4)
  screen.move(0, 62)
  screen.text("cut " .. params:string("cutoff"))
  screen.move(127, 62)
  screen.text_right("acc " .. params:string("accent"))
  screen.update()
end
