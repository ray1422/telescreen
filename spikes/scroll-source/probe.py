#!/usr/bin/env python3
"""Spike: compare wheel scroll and button scroll in a browser on KWin.

Run this on the Linux computer. Open the printed URL in the browser. Keep the
pointer on the page. Each button on the page runs one test.

The script uses only the standard library. It makes two uinput devices:
- A wheel device. It sends the same events as the product (IN-15).
- A stick device with no wheel. libinput gives it button scrolling by default.
"""

import fcntl
import json
import os
import signal
import struct
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = 8766
MAX_RUN_TIME_S = 20 * 60

UI_SET_EVBIT = 0x40045564
UI_SET_KEYBIT = 0x40045565
UI_SET_RELBIT = 0x40045566
UI_DEV_SETUP = 0x405C5503
UI_DEV_CREATE = 0x5501
UI_DEV_DESTROY = 0x5502

EV_SYN, EV_KEY, EV_REL = 0, 1, 2
REL_X, REL_Y, REL_HWHEEL, REL_WHEEL = 0, 1, 6, 8
REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES = 11, 12
BTN_LEFT, BTN_RIGHT, BTN_MIDDLE = 0x110, 0x111, 0x112
BUS_VIRTUAL = 6

# libinput: the button scroll timeout is 200 ms. Wait longer.
ARM_WAIT_S = 0.25
# libinput: the scroll threshold and the direction lock threshold are 5.
ARM_COUNTS = 5
TICK_S = 0.013


class Device:
    def __init__(self, name, product, rel_codes):
        self.fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
        fcntl.ioctl(self.fd, UI_SET_EVBIT, EV_KEY)
        fcntl.ioctl(self.fd, UI_SET_EVBIT, EV_REL)
        for key in (BTN_LEFT, BTN_RIGHT, BTN_MIDDLE):
            fcntl.ioctl(self.fd, UI_SET_KEYBIT, key)
        for code in rel_codes:
            fcntl.ioctl(self.fd, UI_SET_RELBIT, code)
        setup = struct.pack("<4H80sI", BUS_VIRTUAL, 0x1234, product, 1, name.encode(), 0)
        fcntl.ioctl(self.fd, UI_DEV_SETUP, setup)
        fcntl.ioctl(self.fd, UI_DEV_CREATE)

    def emit(self, *events):
        data = b"".join(struct.pack("<qqHHi", 0, 0, *e) for e in events)
        data += struct.pack("<qqHHi", 0, 0, EV_SYN, 0, 0)
        os.write(self.fd, data)

    def close(self):
        fcntl.ioctl(self.fd, UI_DEV_DESTROY)
        os.close(self.fd)


class Probe:
    def __init__(self):
        self.lock = threading.Lock()
        self.wheel = Device("Telescreen spike wheel", 0x5701, [REL_X, REL_Y, REL_WHEEL, REL_HWHEEL, REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES])
        self.stick = Device("Telescreen spike stick", 0x5702, [REL_X, REL_Y])
        self.armed = False

    def arm(self):
        """Hold the middle button, and go past the libinput thresholds.

        The two opposite moves give a net movement of zero. After them, both axes
        scroll with no threshold until the button goes up. A button up after a move
        gives no click.
        """
        if self.armed:
            return
        self.stick.emit((EV_KEY, BTN_MIDDLE, 1))
        time.sleep(ARM_WAIT_S)
        self.stick.emit((EV_REL, REL_X, ARM_COUNTS), (EV_REL, REL_Y, ARM_COUNTS))
        self.stick.emit((EV_REL, REL_X, -ARM_COUNTS), (EV_REL, REL_Y, -ARM_COUNTS))
        self.armed = True

    def disarm(self):
        if self.armed:
            self.stick.emit((EV_KEY, BTN_MIDDLE, 0))
            self.armed = False

    def run(self, mode):
        with self.lock:
            if mode.startswith("stick"):
                self.arm()
                time.sleep(0.3)
            sent = 0
            for per_tick, ticks, event in self.pattern(mode):
                for _ in range(ticks):
                    event(per_tick)
                    sent += abs(per_tick)
                    time.sleep(TICK_S)
            return sent

    def pattern(self, mode):
        wheel_v = lambda n: self.wheel.emit((EV_REL, REL_WHEEL_HI_RES, -n))
        stick_v = lambda n: self.stick.emit((EV_REL, REL_Y, n))
        stick_h = lambda n: self.stick.emit((EV_REL, REL_X, n))
        return {
            "wheel": [(6, 120, wheel_v)],
            "stick": [(6, 120, stick_v)],
            "stick-slow": [(1, 240, stick_v)],
            "stick-fast": [(30, 60, stick_v)],
            "stick-horizontal": [(1, 240, stick_h)],
        }[mode]

    def close(self):
        self.disarm()
        time.sleep(0.1)
        self.wheel.close()
        self.stick.close()


PAGE = """<!doctype html><meta charset=utf-8><title>Scroll probe</title>
<style>
body{margin:0;font:15px sans-serif}
#bar{position:fixed;top:12px;left:12px;background:#fff;color:#000;padding:10px;border:1px solid #888;z-index:1;max-width:760px}
button{margin:3px;padding:8px}
pre{white-space:pre-wrap;margin:6px 0 0}
#room{width:4000000px;height:4000000px;background:repeating-linear-gradient(0deg,#ddd 0 20px,#eee 20px 40px)}
</style>
<div id=bar>
<button data-mode=wheel>1 wheel</button>
<button data-mode=stick>2 stick</button>
<button data-mode=stick-slow>3 stick slow</button>
<button data-mode=stick-fast>4 stick fast</button>
<button data-mode=stick-horizontal>5 stick horizontal</button>
<button id=finish>6 finish</button>
<pre id=out>Keep the pointer on this page.</pre>
</div>
<div id=room></div>
<script>
const out=document.getElementById('out');
const buttonEvents=[];
for(const type of ['mousedown','mouseup','auxclick'])
  addEventListener(type,e=>{buttonEvents.push({type,button:e.button,t:performance.now()})},true);
addEventListener('auxclick',e=>e.preventDefault(),true);
addEventListener('mousedown',e=>{if(e.button===1)e.preventDefault()},true);
const post=(path,body)=>fetch(path,{method:'POST',body:JSON.stringify(body||{})}).then(r=>r.json());
const at=(curve,value)=>{const p=curve.find(c=>Math.abs(c.d)>=Math.abs(value));return p?p.t:null};
async function run(mode){
  const axis=mode==='stick-horizontal'?'scrollX':'scrollY';
  scrollTo(2000000,2000000);
  await new Promise(r=>setTimeout(r,300));
  const home=window[axis],wheels=[],frames=[];let on=true;
  const onWheel=e=>wheels.push({t:performance.now(),dx:e.deltaX,dy:e.deltaY,mode:e.deltaMode});
  addEventListener('wheel',onWheel,{passive:true});
  const tick=()=>{frames.push({t:performance.now(),d:window[axis]-home});if(on)requestAnimationFrame(tick)};
  requestAnimationFrame(tick);
  const before=buttonEvents.length;
  const {sent}=await post('/run?mode='+mode);
  await new Promise(r=>setTimeout(r,1500));
  on=false;removeEventListener('wheel',onWheel);
  if(!wheels.length){out.textContent=mode+': no wheel event arrived';post('/result',{mode,sent,wheels:0});return}
  const key=mode==='stick-horizontal'?'dx':'dy';
  const total=frames[frames.length-1].d;
  let sum=0;const fed=wheels.map(w=>{sum+=w[key];return{t:w.t,d:sum}});
  const half=total/2;
  const lastWheel=wheels[wheels.length-1].t;
  let lastMove=frames[0].t;
  for(let i=1;i<frames.length;i++)if(frames[i].d!==frames[i-1].d)lastMove=frames[i].t;
  const firstMove=(frames.find(f=>f.d!==0)||{}).t;
  const r={mode,sent,total,pxPerCount:+(Math.abs(total)/sent).toFixed(3),wheelEvents:wheels.length,
    wheelSum:+sum.toFixed(1),deltaMode:wheels[0].mode,
    firstMoveMs:firstMove?+(firstMove-wheels[0].t).toFixed(1):null,
    lagAtHalfMs:+((at(frames,half)||0)-(at(fed,half*(sum/total))||0)).toFixed(1),
    settleAfterLastWheelMs:+(lastMove-lastWheel).toFixed(1),
    buttonEvents:buttonEvents.slice(before).filter(e=>e.button===1)};
  out.textContent=JSON.stringify(r,null,0);
  post('/result',r);
}
document.querySelectorAll('button[data-mode]').forEach(b=>b.onclick=()=>run(b.dataset.mode));
document.getElementById('finish').onclick=async()=>{
  const before=buttonEvents.length;
  await post('/disarm');
  await new Promise(r=>setTimeout(r,1000));
  const middle=buttonEvents.slice(before).filter(e=>e.button===1);
  await post('/result',{mode:'finish',buttonEvents:middle});
  out.textContent='Finished. Middle button events after the release: '+middle.length;
  post('/quit');
};
</script>"""


def main():
    probe = Probe()
    done = threading.Event()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def reply(self, body, kind="application/json"):
            data = body.encode()
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            self.reply(PAGE, "text/html; charset=utf-8")

        def do_POST(self):
            length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(length)
            if self.path.startswith("/run?mode="):
                sent = probe.run(self.path.split("=", 1)[1])
                self.reply(json.dumps({"sent": sent}))
            elif self.path == "/disarm":
                probe.disarm()
                self.reply("{}")
            elif self.path == "/result":
                print(body.decode(), flush=True)
                self.reply("{}")
            elif self.path == "/quit":
                self.reply("{}")
                done.set()
            else:
                self.send_error(404)

    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    signal.signal(signal.SIGTERM, lambda *_: done.set())
    signal.signal(signal.SIGHUP, lambda *_: done.set())
    print(f"open http://127.0.0.1:{PORT}/", flush=True)
    done.wait(MAX_RUN_TIME_S)
    probe.close()
    print("closed", flush=True)


if __name__ == "__main__":
    sys.exit(main())
