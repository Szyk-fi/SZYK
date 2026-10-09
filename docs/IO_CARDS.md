# I/O cards

Portamax's jacks live on small plug-in cards instead of the main board.
There are **two slots, A and B**, side by side on the back edge, taking
about 80% of it, with the L1/R1 buttons beside them. Both slots are wired
identically, so any card works in either one.

- **Repair:** a broken jack or pot means a new card, not a new unit.
- **Choice:** the same main board can carry audio, CV, MIDI, or a card
  someone else designs.
- **What stays on the main board:** headphones (DAC + OPA1622) and the
  speaker. Line in/out and MIDI moved to cards in the 9 Oct schematic
  (`Kicad Projects/Portamax v2`).

This document is the interface: connector, pinout, MCU pins, the ID
EEPROM, power and the boot rules. The firmware side is `src/io_cards.rs`,
which the sim already runs with the same two slots (`SLOTS`,
`SLOT_LANES`). The **Cards** app shows what's in each slot.

## Design rules

1. **Only digital signals cross the connector.** Each card has its own
   converters: a codec for audio, an ADC/DAC for CV, an opto for MIDI.
2. **Every card identifies itself.** A 24C02 EEPROM on the card says
   what it is. Nothing on a slot is powered or configured until that
   EEPROM has been read and checked.
3. **No hot-plug in rev A.** Cards are read once at power-up. Pulling or
   inserting one while running mutes the outputs, switches that slot
   off, and asks for a restart.
4. **The connector carries signals, not force.** The jacks are fixed to
   the case wall, and the card is screwed down. A cable being yanked
   loads the case, never the socket.

## Connector

**M.2 Key E socket, 2230 cards (22 × 30 mm), one screw each.**

- Power and ground stay on the M.2 Key E standard positions: +3.3 V on
  pins 2, 4, 72 and 74; GND on 1, 7, 33, 39, 45, 51, 57, 63, 69 and 75
  (plus 11, 15, 20, 23, 37 and 42 beside the clocks). Everything else is
  Portamax-specific, so a stray Wi-Fi card only ever sees 3.3 V.
- The slot's +5 V stays off unless the EEPROM reads back as a Portamax
  card.
- Pins 24–31 are the Key E notch, which leaves 67 contacts.
- The socket footprint is not in KiCad's library: import the chosen
  socket's footprint (e.g. a TE or Amphenol Key E, 2230 standoff).

## Audio clocking

One clock domain for the whole unit:

- The 24.576 MHz oscillator (Y2) feeds the DAC and, through its own
  33 Ω resistor, each slot's **SAI_MCLK**.
- The PCM5122 DAC is the I2S master. Its BCK and LRCK (AUDIO_BCK,
  AUDIO_LRCK) go to the MCU, the speaker amp, the Bluetooth module and,
  through 33 Ω per slot, each slot's **SAI_SCK** and **SAI_FS**.
- The MCU's audio peripherals are all slaves on those clocks. So the
  cards, the DAC, the speaker and Bluetooth all run at the same sample
  rate (48/96/192 kHz; 44.1 kHz material is resampled).
- Card codecs run as I2S slaves with MCLK = 24.576 MHz.

## Pinout (one slot)

Directions are seen from the card toward the MCU. The MCU balls for each
slot are in the next table.

| Pin Number | Pin Name (on IC) | Global Label | Direction (from IC to MCU) | MCU Pin Name (destination) | STM32 Pin Type |
|---|---|---|---|---|---|
| 1 | GND | GND | Power | — | Power |
| 2 | 3V3 | +3V3 | Power | — | Power |
| 3 | USB_DP | — | — | reserved (USB, not fitted on rev A) | — |
| 4 | 3V3 | +3V3 | Power | — | Power |
| 5 | USB_DN | — | — | reserved (USB, not fitted on rev A) | — |
| 6 | PRESENT# | SLOTx_PRESENT_N | Output | see slot table | GPIO input (10 kΩ pull-up) |
| 7 | GND | GND | Power | — | Power |
| 8 | RSVD | — | — | — | — |
| 9 | SAI_MCLK | SLOTx_SAI_MCLK | Input | from Y2 24.576 MHz via 33 Ω | — |
| 10 | RSVD | — | — | — | — |
| 11 | GND | GND | Power | — | Power |
| 12 | RESET# | SLOTx_RESET_N | Input | see slot table | GPIO output (10 kΩ pull-down: card held in reset at boot) |
| 13 | SAI_SCK | SLOTx_SAI_SCK | Input | AUDIO_BCK via 33 Ω | — |
| 14 | INT# | SLOTx_INT_N | Output | see slot table | GPIO input, EXTI (10 kΩ pull-up) |
| 15 | GND | GND | Power | — | Power |
| 16 | I2C_SCL | SLOTx_I2C_SCL | Input | TCA9548A channel (A = 0, B = 1) | via mux (4.7 kΩ pull-up) |
| 17 | SAI_FS | SLOTx_SAI_FS | Input | AUDIO_LRCK via 33 Ω | — |
| 18 | I2C_SDA | SLOTx_I2C_SDA | Bidirectional | TCA9548A channel (A = 0, B = 1) | via mux (4.7 kΩ pull-up) |
| 19 | SAI_SD_OUT | SLOTx_SAI_SD_OUT | Input | see slot table | AF output |
| 20 | GND | GND | Power | — | Power |
| 21 | SAI_SD_IN | SLOTx_SAI_SD_IN | Output | see slot table | AF input |
| 22 | GPIO0 | SLOTx_GPIO0 | Bidirectional | see slot table | GPIO |
| 23 | GND | GND | Power | — | Power |
| 24–31 | key | — | — | — | — |
| 32 | GPIO1 | SLOTx_GPIO1 | Bidirectional | see slot table | GPIO |
| 33 | GND | GND | Power | — | Power |
| 34 | GPIO2 | SLOTx_GPIO2 | Bidirectional | see slot table | GPIO |
| 35 | SPI_SCK | SPI_CARD_SCK | Input | U12 B12 (PE12) | SPI4_SCK, shared |
| 36 | GPIO3 | SLOTx_GPIO3 | Bidirectional | see slot table | GPIO |
| 37 | GND | GND | Power | — | Power |
| 38 | UART_TX | SLOTx_UART_TX | Input | see slot table | USART TX |
| 39 | GND | GND | Power | — | Power |
| 40 | UART_RX | SLOTx_UART_RX | Output | see slot table | USART RX (10 kΩ pull-up) |
| 41 | SPI_MOSI | SPI_CARD_MOSI | Input | U12 B11 (PE14) | SPI4_MOSI, shared |
| 42 | GND | GND | Power | — | Power |
| 43 | SPI_MISO | SPI_CARD_MISO | Output | U12 A11 (PE13) | SPI4_MISO, shared: cards must tri-state it when not selected |
| 44, 46, 48, 50, 52–56, 58–61, 65, 67, 70, 71, 73 | RSVD | — | — | — | — |
| 47 | SPI_CS0 | SLOTx_SPI_CS0 | Input | see slot table | GPIO output |
| 49 | SPI_CS1 | SLOTx_SPI_CS1 | Input | see slot table | GPIO output |
| 45, 51, 57, 63, 69, 75 | GND | GND | Power | — | Power |
| 62, 64, 66, 68 | 5V_SW | SLOTx_5V | Power | slot load switch | Power |
| 72, 74 | 3V3 | +3V3 | Power | — | Power |

The reserved pins are kept for rev B (a second audio data pair, USB for a
USB-host card). Leave them unconnected on both sides.

## MCU pins per slot

Checked against CubeMX's pin database for the STM32N647X0HxQ.

| Signal | Slot A ball (pin) | Slot A function | Slot B ball (pin) | Slot B function |
|---|---|---|---|---|
| SAI_SD_OUT | E16 (PB7) | SAI1_SD_A | T14 (PG8) | I2S2_SDO |
| SAI_SD_IN | R14 (PA3) | SAI1_SD_B | G14 (PD11) | I2S2_SDI |
| Clock inputs on the MCU side | E17 (PB6) SAI1_SCK_A, E15 (PB0) SAI1_FS_A | slave | U7 (PA12) I2S2_CK, R10 (PB12) I2S2_WS | slave |
| UART_TX | P7 (PA9) | USART1_TX | U10 (PA0) | UART4_TX |
| UART_RX | R7 (PA10) | USART1_RX | P9 (PA1) | UART4_RX |
| SPI_CS0 / CS1 | F17 (PE1) / F16 (PE0) | GPIO | A14 (PD1) / B16 (PE10) | GPIO |
| GPIO0–3 | A15 (PE8), B13 (PE7), R3 (PF0), A12 (PE15) | GPIO | E14 (PD8), L4 (PF10), D10 (PE4), P6 (PG14) | GPIO |
| PRESENT# | L3 (PG6) | GPIO in | N3 (PF8) | GPIO in |
| RESET# | R13 (PG0) | GPIO out | P4 (PF1) | GPIO out |
| INT# | E11 (PE11) | EXTI11 | H14 (PD9) | EXTI9 |
| 5V enable | T13 (PG9) | GPIO out | T9 (PG12) | GPIO out |
| I2C | TCA9548A (0x70 on I2C1) channel 0 | — | channel 1 | — |

Shared: SPI4 (B12, B11, A11) for both slots; CARD_5V_FLT_N on R1 (PG3) is
the wired-OR fault line from both load switches.

## Power

- **+3.3 V** is always on, from the standard pins. It powers only the
  EEPROM and logic.
- **+5 V** comes through a TPS2051C load switch per slot (fixed current
  limit, 500 mA continuous, active-high enable with a 100 kΩ pull-down, so
  it is off at boot).
  - It is only switched on after the EEPROM checks out, and the card's
    declared current fits the budget.
  - Slots are powered A then B. A card that would take the total past the
    +5 V budget (`FIVE_V_BUDGET_MA`, a 1 A placeholder until the power
    design fixes it) stays off.
  - Both switches' fault outputs share CARD_5V_FLT_N.
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
- an unknown kind or lane;
- a card needing a lane its slot doesn't have;
- a card that would go over the +5 V budget.

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
| MIDI out | a note-bus instrument, "Slot A MIDI Out" | every Plays row and Portal's Notes page |
| MIDI in | a note-bus source, "Slot A MIDI In" | routable to any instrument |
| Audio out | the main outputs | everything already goes there |

**In the sim:**
- The slots are set in `saves/io_slots.json`. The default is A = Audio
  2x2 (line in/out) and B = MIDI DIN. An older six-slot save still loads;
  entries past B are ignored.
- Changing a slot in the Cards app applies at the next start, as on the
  device.
- Card inputs read silence, except the first audio input, which carries
  the computer's audio input.
