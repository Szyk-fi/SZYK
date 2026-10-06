// The firmware is an Arduino sketch: its .ino files are concatenated into one
// translation unit (the main sketch first, the others alphabetically, with
// generated prototypes). Order here puts every definition before its use, so no
// prototypes are needed.
#include <Arduino.h>
// The Arduino build includes the main sketch's headers first, which is what
// makes the apps' own includes resolve; do the same.
#include <EEPROM.h>
#include "fw/OC_apps.h"
#include "fw/OC_core.h"
#include "fw/OC_DAC.h"
#include "fw/OC_debug.h"
#include "fw/OC_gpio.h"
#include "fw/OC_ADC.h"
#include "fw/OC_calibration.h"
#include "fw/OC_digital_inputs.h"
#include "fw/OC_menus.h"
#include "fw/OC_ui.h"
#include "fw/OC_version.h"
#include "fw/OC_options.h"
#include "fw/src/drivers/display.h"
#include "fw/util/util_debugpins.h"
#include "fw/VBiasManager.h"

#include "prototypes.h"
#include "fw/o_c_REV.ino"

#include "fw/APP_ASR.ino"
#include "fw/APP_AUTOMATONNETZ.ino"
#include "fw/APP_A_SEQ.ino"
#include "fw/APP_BBGEN.ino"
#include "fw/APP_BYTEBEATGEN.ino"
#include "fw/APP_CHORDS.ino"
#include "fw/APP_DQ.ino"
#include "fw/APP_ENVGEN.ino"
#include "fw/APP_H1200.ino"
#include "fw/APP_LORENZ.ino"
#include "fw/APP_POLYLFO.ino"
#include "fw/APP_QQ.ino"
#include "fw/APP_REFS.ino"
#include "fw/OC_apps.ino"
#include "fw/OC_calibration.ino"
