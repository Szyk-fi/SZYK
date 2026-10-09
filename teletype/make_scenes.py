"""Writes the bundled Teletype scenes in the module's own text format.

Run from the repo root: python3 teletype/make_scenes.py
"""
from pathlib import Path

SCENES = {
    "01 pattern melody": (
        "PATTERN MELODY\nThe metro walks pattern 0 into CV 1 and pulses TR 1.\n"
        "Pad 1 reverses the pattern, 2 shuffles it, 3 transposes up a fourth.\n"
        "On the grid: eight faders are the eight steps (press a key to set its\n"
        "note), the row under them is the playhead, and the bottom row's three\n"
        "buttons are pads 1-3.",
        {
            "1": ["P.REV"],
            "2": ["P.SHUF"],
            "3": ["X WRAP ADD X 5 0 11"],
            "8": ["L 0 7: P I G.FDR.V I"],
            "M": [
                "G.CLR; G.LED P.I 6 15",
                "CV 1 N ADD 48 ADD X P.NEXT",
                "TR.P 1",
                "EVERY 8: CV 2 N ADD 36 X",
                "EVERY 8: TR.P 2",
                "PROB 20: TR.P 3",
            ],
            "I": [
                "M 180; TR.TIME 1 120; TR.TIME 2 1200",
                "CV 3 N 79; TR.TIME 3 40",
                "G.GFDR.RN 0 0 12",
                "L 0 7: G.FDR I I 0 1 6 1 12 8",
                "L 0 7: G.FDR.V I P I",
                "G.BTX 0 0 7 4 1 0 3 1 3 1",
            ],
        },
        [[0, 3, 7, 10, 12, 10, 7, 5]],
    ),
    "02 euclid drums": (
        "EUCLID DRUMS\nFour euclidean rhythms on TR 1-4 over a 16-step count in X.\n"
        "Pad 1 changes the fills.\n"
        "On the grid: the top two faders set the kick and snare fills, the third\n"
        "the speed; the four buttons under them mute tracks 1-4; the row below is\n"
        "the 16-step count.",
        {
            "1": ["G.FDR.N 0 RRAND 2 6; G.FDR.N 1 RRAND 3 9", "CV 4 N ADD 55 RRAND 0 12"],
            "M": [
                "X WRAP ADD X 1 0 15; G.CLR; G.LED X 6 15",
                "A G.FDR.N 0; B G.FDR.N 1; M SCALE 0 15 400 60 G.FDR.N 2",
                "IF AND ER A 16 X EZ G.BTN.V 0: TR.P 1",
                "IF AND ER B 16 X EZ G.BTN.V 1: TR.P 2",
                "IF AND ER 3 8 SUB X 2 EZ G.BTN.V 2: TR.P 3",
                "IF AND EZ MOD X 4 EZ G.BTN.V 3: TR.P 4",
            ],
            "I": [
                "CV 1 N 36; CV 2 N 55; CV 3 N 67",
                "L 1 4: TR.TIME I 60",
                "L 0 1: G.FDR I 0 I 16 1 0 9 0",
                "G.FDR 2 0 2 16 1 0 15 0; CV 4 N 60",
                "G.FDR.N 0 4; G.FDR.N 1 5; G.FDR.N 2 8",
                "G.BTX 0 0 4 4 1 1 3 0 4 1",
            ],
        },
        [],
    ),
    "03 drunk walk": (
        "DRUNK WALK\nA melody that wanders a step at a time, and a bass note\n"
        "every eighth tick. Pad 1 sends the walk back to the middle.\n"
        "On the grid: the walk is drawn as it goes, one column per tick. The top\n"
        "fader sets how far it may wander, the button under it is pad 1.",
        {
            "1": ["DRUNK 12"],
            "8": ["DRUNK.MAX ADD 2 MUL 2 G.FDR.N 0"],
            "M": [
                "X WRAP ADD X 1 0 15; A QT DRUNK 2",
                "CV 1 N ADD 48 A; TR.P 1",
                "L 2 7: G.LED X I 0",
                "G.LED X SUB 7 DIV A 4 15",
                "EVERY 8: CV 2 N ADD 36 RAND 12",
                "EVERY 8: TR.P 2",
            ],
            "I": [
                "M 150; DRUNK.MAX 24; DRUNK 12",
                "TR.TIME 2 900; CV.SLEW 1 20",
                "G.FDR 0 0 0 16 1 0 11 8; G.FDR.N 0 11",
                "G.BTN 0 0 1 4 1 0 3 1",
            ],
        },
        [],
    ),
    "04 pad scripts": (
        "PAD SCRIPTS\nNothing runs on its own: play scripts 1-8 from the SCRIPTS pads.\n"
        "1-3 and 5 are notes, 4 is an arpeggio, 6 a random bass,\n"
        "7 changes the note length and 8 starts and stops the metro.\n"
        "On the grid: the top row of big pads plays scripts 1-4, the row under it\n"
        "plays the random bass (script 6), and the fader on the bottom row sets the\n"
        "note length.",
        {
            "1": ["CV 1 N 48; TR.P 1"],
            "2": ["CV 1 N 55; TR.P 1"],
            "3": ["CV 1 N 60; TR.P 1"],
            "4": ["$ 1", "DEL 120: $ 2", "DEL 240: $ 3", "DEL 360: $ 5"],
            "5": ["CV 1 N 64; TR.P 1"],
            "6": ["CV 2 N RRAND 36 48", "TR.P 2"],
            "7": ["TR.TIME 1 ADD 40 MUL 25 G.FDR.N 0"],
            "8": ["M.ACT EZ M.ACT"],
            "M": ["PROB 40: $ 6"],
            "I": [
                "M 250; TR.TIME 1 150; TR.TIME 2 400; M.ACT 0",
                "L 1 4: G.BTN I MUL 3 I 0 3 3 0 3 I",
                "G.BTX 8 3 3 3 3 0 3 6 4 1",
                "G.FDR 0 0 7 16 1 0 15 7; G.FDR.N 0 4",
            ],
        },
        [],
    ),
    "05 param arp": (
        "PARAM ARP\nThe Param knob (or the right hand) sets the speed,\n"
        "IN (or the left hand) transposes. Pad 1 picks a new chord.\n"
        "On the grid: six wide faders paint the chord's notes (drag to change),\n"
        "the dots under them follow the arpeggio, and the bottom row picks a\n"
        "new chord.",
        {
            "1": ["L 0 5: P I ADD PN 1 I MUL 3 RAND 1", "L 0 5: G.FDR.V I P I"],
            "8": ["L 0 5: P I G.FDR.V I"],
            "M": [
                "G.CLR; G.LED MUL 2 P.I 6 15",
                "M SCALE 0 16383 400 60 PARAM",
                "J SCALE 0 16383 0 24 IN",
                "CV 1 N ADD 48 ADD J P.NEXT",
                "TR.P 1",
            ],
            "I": [
                "M 200; TR.TIME 1 80",
                "G.GFDR.RN 0 0 24",
                "L 0 5: G.FDR I MUL 2 I 0 2 6 1 24 8",
                "L 0 5: G.FDR.V I P I",
                "G.BTN 0 0 7 16 1 0 3 1",
            ],
        },
        [[0, 4, 7, 11, 14, 11], [0, 4, 7, 11, 14, 11]],
    ),
    "06 grid steps": (
        "GRID STEPS\nA four-track step sequencer on the grid (the Grid app or a\n"
        "real grid): rows 1-4 are the tracks, press a key to toggle a step.\n"
        "Row 7 is a fader that sets the speed. F3 starts it.",
        {
            "1": ["M G.FDRV"],
            "M": [
                "G.CLR; X WRAP + X 1 0 15",
                "G.REC X 0 1 4 -2 -2",
                "IF G.BTN.V X: TR.P 1",
                "IF G.BTN.V + X 16: TR.P 2",
                "IF G.BTN.V + X 32: TR.P 3",
                "IF G.BTN.V + X 48: TR.P 4",
            ],
            "I": [
                "G.BTX 0 0 0 1 1 1 3 0 16 4",
                "G.FDR 0 0 6 16 1 0 3 1; G.GFDR.RN 0 300 60",
                "G.FDR.N 0 8; M G.FDR.V 0",
                "CV 1 N 36; CV 2 N 48; CV 3 N 55",
                "CV 4 N 62; G.BTN.V 0 1; G.BTN.V 8 1",
                "G.BTN.V 20 1; G.BTN.V 28 1; G.BTN.V 42 1",
            ],
        },
        [],
    ),
}


def write(name, desc, scripts, pats):
    out = [desc, ""]
    for s in ["1", "2", "3", "4", "5", "6", "7", "8", "M", "I"]:
        out.append("#" + s)
        out.extend(scripts.get(s, []))
        out.append("")
    pats = (pats + [[], [], [], []])[:4]
    out.append("#P")
    out.append("\t".join(str(len(p)) for p in pats))
    out.append("\t".join("1" for _ in pats))
    out.append("\t".join("0" for _ in pats))
    out.append("\t".join("63" for _ in pats))
    out.append("")
    for i in range(64):
        out.append("\t".join(str(p[i] if i < len(p) else 0) for p in pats))
    folder = Path(__file__).with_name("scenes")
    folder.mkdir(exist_ok=True)
    folder.joinpath(name.replace(" ", "_") + ".txt").write_text("\n".join(out) + "\n")


for name, (desc, scripts, pats) in SCENES.items():
    write(name, desc, scripts, pats)
