# I/O cards

Portamax's jacks live on small plug-in cards instead of the main board.

- **Repair:** a broken jack or pot means a new card, not a new unit.
- **Choice:** the same main board can carry audio, CV, MIDI, or a card
  someone else designs.

This document is the interface: connector, pinout, the ID EEPROM, power
and the boot rules. The firmware side is `src/io_cards.rs`, which the
sim already runs. The **Cards** app shows what's in each slot.

## Design rules

1. **Only digital signals cross the connector.** Each card has its own
   converters: a codec for audio, an ADC/DAC for CV, an opto for MIDI.
   Analog audio passing through a card-edge contact next to a 600 GOPS
   NPU would pick up its switching noise.
2. **Every card identifies itself.** A 24C02 EEPROM on the card says
   what it is. Nothing on a slot is powered or configured until that
   EEPROM has been read and checked.
3. **No hot-plug in rev A.** Cards are read once at power-up. Pulling or
   inserting one while running mutes the outputs, switches that slot
   off, and asks for a restart.
4. **The connector carries signals, not force.** The jacks are fixed to
   the front panel, and the card is bolted to the panel bracket. A cable
   being yanked loads the panel, never the socket.

## Connector

**M.2 Key E, 2230 cards (22 × 30 mm), one screw each.**

Why M.2 rather than SO-DIMM:
- **Durability:** it is rated for many more mating cycles.
- **Retention:** the screw holds the card down.
- **Size and cost:** it is small, cheap and widely stocked.
- **Fabrication:** 2230 boards are an ordinary JLCPCB job with gold
  fingers and a bevel. Both are paid options there.

**Safety with stray cards:**
- Power and ground stay on the M.2 Key E standard positions:
  - +3.3 V on pins 2, 4, 72 and 74.
  - GND on pins 1, 7, 33, 39, 45, 51, 57, 63, 69 and 75.
- Everything else is Portamax-specific.
- If someone plugs a Wi-Fi card into a slot, it only ever sees 3.3 V on
  its normal pins and 3.3 V logic elsewhere.
- The slot's +5 V stays off unless the EEPROM reads back as a Portamax
  card.

**Positions:** pins 24–31 are the Key E notch, which leaves 67 contacts.

## Pinout (one slot)

Directions are seen from the card toward the MCU.

The MCU column names the peripheral signal for slot A. The ball/GPIO
names are left for CubeMX, because they must be picked alongside the
xSPI RAM, the display and the SD card, and I haven't checked them against
the STM32N647's alternate-function table. Fill them in from there.

| Pin Number | Pin Name (on IC) | Global Label | Direction (from IC to MCU) | MCU Pin Name (destination) | STM32 Pin Type |
|---|---|---|---|---|---|
| 1 | GND | GND | Power | — | Power |
| 2 | 3V3 | +3V3 | Power | — | Power |
| 3 | USB_DP | SLOTA_USB_DP | Bidirectional | reserved (USB, not fitted on rev A) | — |
| 4 | 3V3 | +3V3 | Power | — | Power |
| 5 | USB_DN | SLOTA_USB_DN | Bidirectional | reserved (USB, not fitted on rev A) | — |
| 6 | PRESENT# | SLOTA_PRESENT_N | Output | GPIO (TBD) | GPIO input, pull-up |
| 7 | GND | GND | Power | — | Power |
| 8 | RSVD | — | — | — | — |
| 9 | SAI_MCLK | SLOTA_SAI_MCLK | Input | SAI1_MCLK_A | AF push-pull |
| 10 | RSVD | — | — | — | — |
| 11 | GND | GND | Power | — | Power |
| 12 | RESET# | SLOTA_RESET_N | Input | GPIO (TBD) | GPIO output, push-pull |
| 13 | SAI_SCK | SLOTA_SAI_SCK | Input | SAI1_SCK_A | AF push-pull |
| 14 | INT# | SLOTA_INT_N | Output | GPIO (TBD), EXTI | GPIO input, pull-up |
| 15 | GND | GND | Power | — | Power |
| 16 | I2C_SCL | SLOTA_I2C_SCL | Input | I2C mux channel 0 (TCA9548A) | via mux |
| 17 | SAI_FS | SLOTA_SAI_FS | Input | SAI1_FS_A | AF push-pull |
| 18 | I2C_SDA | SLOTA_I2C_SDA | Bidirectional | I2C mux channel 0 (TCA9548A) | via mux |
| 19 | SAI_SD_OUT | SLOTA_SAI_SD_OUT | Input | SAI1_SD_A (transmit) | AF push-pull |
| 20 | GND | GND | Power | — | Power |
| 21 | SAI_SD_IN | SLOTA_SAI_SD_IN | Output | SAI1_SD_B (receive, synchronous) | AF input |
| 22 | GPIO0 | SLOTA_GPIO0 | Bidirectional | GPIO (TBD) | GPIO |
| 23 | GND | GND | Power | — | Power |
| 24–31 | key | — | — | — | — |
| 32 | GPIO1 | SLOTA_GPIO1 | Bidirectional | GPIO (TBD) | GPIO |
| 33 | GND | GND | Power | — | Power |
| 34 | GPIO2 | SLOTA_GPIO2 | Bidirectional | GPIO (TBD) | GPIO |
| 35 | SPI_SCK | SPI_CARD_SCK | Input | SPIx_SCK (shared by all slots) | AF push-pull |
| 36 | GPIO3 | SLOTA_GPIO3 | Bidirectional | GPIO (TBD) | GPIO |
| 37 | GND | GND | Power | — | Power |
| 38 | UART_TX | SLOTA_UART_TX | Input | USARTx_TX | AF push-pull |
| 39 | GND | GND | Power | — | Power |
| 40 | UART_RX | SLOTA_UART_RX | Output | USARTx_RX | AF input, pull-up |
| 41 | SPI_MOSI | SPI_CARD_MOSI | Input | SPIx_MOSI (shared) | AF push-pull |
| 42 | GND | GND | Power | — | Power |
| 43 | SPI_MISO | SPI_CARD_MISO | Output | SPIx_MISO (shared) | AF input |
| 44 | RSVD | — | — | — | — |
| 45 | GND | GND | Power | — | Power |
| 46 | RSVD | — | — | — | — |
| 47 | SPI_CS0 | SLOTA_SPI_CS0 | Input | GPIO (TBD) | GPIO output, push-pull |
| 48 | RSVD | — | — | — | — |
| 49 | SPI_CS1 | SLOTA_SPI_CS1 | Input | GPIO (TBD) | GPIO output, push-pull |
| 50 | RSVD | — | — | — | — |
| 51 | GND | GND | Power | — | Power |
| 52–56 | RSVD | — | — | — | — |
| 57 | GND | GND | Power | — | Power |
| 58–61 | RSVD | — | — | — | — |
| 62 | 5V_SW | SLOTA_5V | Power | slot load switch | Power |
| 63 | GND | GND | Power | — | Power |
| 64 | 5V_SW | SLOTA_5V | Power | slot load switch | Power |
| 65 | RSVD | — | — | — | — |
| 66 | 5V_SW | SLOTA_5V | Power | slot load switch | Power |
| 67 | RSVD | — | — | — | — |
| 68 | 5V_SW | SLOTA_5V | Power | slot load switch | Power |
| 69 | GND | GND | Power | — | Power |
| 70–71 | RSVD | — | — | — | — |
| 72 | 3V3 | +3V3 | Power | — | Power |
| 73 | RSVD | — | — | — | — |
| 74 | 3V3 | +3V3 | Power | — | Power |
| 75 | GND | GND | Power | — | Power |

Notes:
- Every clock (MCLK, SCK, SPI_SCK) has a ground pin beside it.
- The reserved pins are kept for rev B: a second SAI data pair, USB for
  a USB-host card, and analog-free extras. Leave them unconnected on
  both sides.

### Per-slot peripherals

| Slot | Audio | Control (I2C) | SPI chip selects | UART |
|---|---|---|---|---|
| A | SAI1 block A/B | mux channel 0 | CS0/CS1 A | USART (TBD) |
| B | SAI2 block A/B | mux channel 1 | CS0/CS1 B | USART (TBD) |
| C | SPI in I2S mode, or a third SAI block if the N647 has one free | mux channel 2 | CS0/CS1 C | USART (TBD) |

How the shared buses are split between slots:
- **I2C:** one bus goes through a TCA9548A mux, so every card's EEPROM
  can sit at the same address (0x50). Two identical cards then never
  collide, even though their codecs share a fixed I2C address.
- **SPI:** one bus is shared, with separate chip selects per slot.
- **Before laying out:** confirm in CubeMX how many SAI blocks and
  USARTs are free once the RAM, display, SD card and camera are
  placed. Slot C's audio lane is the one most likely to need the
  fallback.

## Power

- **+3.3 V** is always on, from the standard pins. It powers only the
  EEPROM and logic.
- **+5 V** comes through a current-limited load switch per slot, such as
  a TPS2553, set to about 500 mA.
  - It is only switched on after the EEPROM checks out, and the card's
    declared current fits the budget.
  - The switch also gives the soft start that rev B hot-plug will need.
- **Clean analog rails** are made on the card: an LDO for the codec, a
  charge pump or boost to ±12 V for the CV card.
- The cards' declared currents add up on the Cards screen, so the
  power budget is checked on every boot.

## ID EEPROM

**Part:** a 24C02 (256 bytes) at 0x50 on the slot's mux channel. Tie
WP high on the card, and leave a pad to pull it low for programming.

**Layout, version 1:**

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `PMXC` |
| 4 | 1 | format version (1) |
| 5 | 1 | reserved, 0 |
| 6 | 2 | vendor id (1 = Portamax) |
| 8 | 2 | product id |
| 10 | 1 | hardware revision |
| 11 | 1 | +5 V current, 10 mA units |
| 12 | 4 | serial number |
| 16 | 24 | name, UTF-8, zero-padded |
| 40 | 1 | number of I/O entries, n |
| 41 | 1 | reserved, 0 |
| 42 | 6 × n | I/O entries |
| 42 + 6n | 2 | CRC-16/CCITT-FALSE of everything before it |

All multi-byte values are little-endian. The 256 bytes hold up to 35 I/O
entries.

**Each I/O entry** is six bytes:

| Byte | Field | Values |
|---|---|---|
| 0 | kind | 1 audio in, 2 audio out, 3 CV in, 4 CV out, 5 gate in, 6 gate out, 7 MIDI in, 8 MIDI out |
| 1 | count | number of channels |
| 2 | lane | 0 SAI, 1 SPI, 2 UART, 3 GPIO, 4 I2C |
| 3 | flags | bit 0 = bipolar |
| 4–5 | param | audio: sample rate ÷ 100; CV: full-scale span in mV |

**What the firmware refuses:**
- a blank EEPROM;
- a wrong magic;
- a newer format version;
- a short read;
- a bad CRC;
- an unknown kind or lane.

A refused slot stays unpowered, and the Cards screen says why.

**Making a new card:**
1. Describe it as JSON in `assets/io_cards/`, or in `saves/io_cards/` on
   the SD card for a third-party card. `audio_2x2.json`, `cv_4x4.json`
   and `midi_din.json` are the examples.
2. The same encoder that makes the image the sim reads makes the bytes to
   program into the card. Its round trip is tested.

## What the system does with a card

`IoCards::install` turns each I/O entry into something the existing
buses already understand, so apps need no changes:

| On the card | Becomes | Used from |
|---|---|---|
| Audio in | an audio-bus source, "Slot A Audio In 1-2" | any effect's Source row, Portal, Clouds, Rings' exciter... |
| CV in, gate in | an audio-bus source, "Slot B CV In 1" | Portal cables (Signal, Envelope, Gate, S&H transforms) |
| CV out, gate out | a mod input, "Slot B: CV Out 1" | Turing Machine, Marbles and Tides outputs; Portal destinations; MIDI Learn |
| MIDI out | a note-bus instrument, "Slot C MIDI Out" | every Plays row and Portal's Notes page |
| MIDI in | a note-bus source, "Slot C MIDI In" | routable to any instrument |
| Audio out | the main outputs | everything already goes there |

**In the sim:**
- The slots are set in `saves/io_slots.json`. The default is A = Audio
  2x2, B = CV 4x4, C = MIDI DIN.
- Changing a slot in the Cards app applies at the next start, as on the
  device.
- Card inputs read silence, except the first audio input, which carries
  the computer's audio input.
