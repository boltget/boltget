// Records frames and long tasks for DURATION ms while the list is flung up
// and down at about 3,600 px/s (the same fling as the Chromium run in
// APP-DESIGN), or held still when FLING is false.
(async () => {
  const DURATION = 10000, FLING = __FLING__;
  const list = document.querySelector("[role='listbox']");
  const scroller = (() => { let e = list; while (e && e.scrollHeight <= e.clientHeight + 1) e = e.parentElement; return e ?? list; })();
  const longTasks = [];
  const obs = new PerformanceObserver((l) => { for (const e of l.getEntries()) longTasks.push(Math.round(e.duration)); });
  obs.observe({ type: "longtask", buffered: false });
  const frames = [];
  let last = performance.now(), dir = 1;
  const start = last;
  await new Promise((done) => {
    function tick(now) {
      frames.push(now - last); last = now;
      if (FLING) {
        const max = scroller.scrollHeight - scroller.clientHeight;
        let next = scroller.scrollTop + dir * 3.6 * frames[frames.length - 1];
        if (next >= max || next <= 0) { dir = -dir; next = Math.max(0, Math.min(max, next)); }
        scroller.scrollTop = next;
      }
      if (now - start < DURATION) requestAnimationFrame(tick); else done();
    }
    requestAnimationFrame(tick);
  });
  obs.disconnect();
  frames.shift();
  frames.sort((a, b) => a - b);
  const q = (p) => Math.round(frames[Math.floor(p * (frames.length - 1))] * 10) / 10;
  const interval = q(0.5);
  return {
    frames: frames.length,
    fps: Math.round(frames.length / (DURATION / 1000)),
    median_ms: interval, p95_ms: q(0.95), p99_ms: q(0.99), max_ms: q(1),
    over_33ms: frames.filter((f) => f > 33.4).length,
    over_25ms: frames.filter((f) => f > 25).length,
    dropped: frames.reduce((n, f) => n + Math.max(0, Math.round(f / interval) - 1), 0),
    long_tasks: longTasks.length, longest_task_ms: Math.max(0, ...longTasks),
    rows_in_dom: document.querySelectorAll("[role='option']").length,
    scroll_height: scroller.scrollHeight,
  };
})()
