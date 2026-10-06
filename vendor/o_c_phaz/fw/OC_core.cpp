#include "OC_core.h"

#include <mutex>

/*volatile*/ std::queue<Task> fn_queue;
// host: the deferring ISR and the app loop are two real threads, so the queue is locked
// (the Teensy relied on the loop being the only thing the ISR could interrupt).
static std::mutex fn_queue_mutex;
//volatile bool fn_queue_lock = false;

void OC::CORE::DeferTask(Task func) {
  // This simply ignores Tasks from the ISR while flushing...
  // Hopefully that's more like debouncing or frame drops and not missed clocks...
  //if (!fn_queue_lock) // oh no!
  std::lock_guard<std::mutex> lock(fn_queue_mutex);
  fn_queue.emplace(func);
}
void OC::CORE::FlushTasks() {
  for (;;) {
    Task task;
    {
      std::lock_guard<std::mutex> lock(fn_queue_mutex);
      if (fn_queue.empty()) return;
      task = fn_queue.front();
      fn_queue.pop();
    }
    task();
  }
}

int OC::CORE::FreeRam() {
  return 0;  // host: no Teensy heap to measure
}
