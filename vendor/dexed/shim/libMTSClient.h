// Portamax stand-in for the MTS-ESP client library: no master is ever
// present, so notes use the DX7's own tuning table.
#pragma once
struct MTSClient;
inline bool MTS_HasMaster(MTSClient*) { return false; }
inline double MTS_NoteToFrequency(MTSClient*, int, int) { return 0.0; }
