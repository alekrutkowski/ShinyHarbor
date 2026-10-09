/* ShinyHarbor transport adapter. SockJS itself is vendored under the MIT license. */
(() => {
  "use strict";
  const script = document.currentScript;
  const base = new URL("../__sockjs__" + (script.dataset.worker ? "/w=" + script.dataset.worker : ""), script.src).href;
  const disabled = JSON.parse(script.dataset.disabled || "[]");
  const bufferBytes = Number(script.dataset.bufferBytes || 134217728);
  const bufferMessages = Number(script.dataset.bufferMessages || 2048);
  // Count UTF-8 bytes without allocating another large encoded message.
  const utf8Bytes = value => {
    let bytes = 0;
    for (let i = 0; i < value.length; i++) {
      const code = value.charCodeAt(i);
      if (code < 0x80) bytes++;
      else if (code < 0x800) bytes += 2;
      else if (code >= 0xD800 && code <= 0xDBFF && i + 1 < value.length &&
               value.charCodeAt(i + 1) >= 0xDC00 && value.charCodeAt(i + 1) <= 0xDFFF) {
        bytes += 4; i++;
      } else bytes += 3;
    }
    return bytes;
  };
  const protocols = ["websocket", "xhr-streaming", "xdr-streaming", "eventsource",
    "iframe-eventsource", "htmlfile", "iframe-htmlfile", "xhr-polling",
    "xdr-polling", "iframe-xhr-polling", "jsonp-polling"];
  const transports = protocols.filter(name => !disabled.some(p => p === name ||
    (p === "streaming" && (name.endsWith("streaming") || /eventsource|htmlfile/.test(name))) ||
    (p === "polling" && name.endsWith("polling")) ||
    (p === "iframe-eventsource" && name === "eventsource") ||
    (p === "iframe-htmlfile" && name === "htmlfile")));
  const randomId = () => Array.from(crypto.getRandomValues(new Uint8Array(16)),
    n => n.toString(16).padStart(2, "0")).join("");
  const options = {transports, sessionId: randomId};

  // Implements the public numbered-message/multiplex wire format. Messages
  // remain buffered until acknowledged; a recovered transport does not create
  // a new Shiny session or repeat application input handlers.
  class RecoveringSocket extends EventTarget {
    constructor() {
      super();
      this.url = base;
      this.readyState = 0;
      this.bufferedAmount = 0;
      this.id = randomId();
      this.sent = 0;
      this.received = 0;
      this.pending = new Map();
      this.deadline = 0;
      this.connected = false;
      this.replaying = false;
      this.connect(false);
    }
    emit(type, event) {
      if (typeof this["on" + type] === "function") this["on" + type](event);
      this.dispatchEvent(event);
    }
    notice(show) {
      if (!show) { this.banner?.remove(); this.banner = null; return; }
      if (this.banner) return;
      this.banner = document.createElement("div");
      this.banner.setAttribute("role", "status");
      this.banner.dataset.shinyharborReconnect = "true";
      this.banner.textContent = "Connection interrupted. Reconnecting to your session…";
      Object.assign(this.banner.style, {position: "fixed", top: "12px", left: "50%",
        transform: "translateX(-50%)", zIndex: "2147483647", padding: "12px 20px",
        borderRadius: "8px", background: "#173343", color: "white", font: "14px system-ui",
        boxShadow: "0 4px 20px #0003", maxWidth: "90vw"});
      document.body.append(this.banner);
    }
    connect(resume) {
      if (this.readyState >= 2) return;
      this.replaying = resume;
      const socket = this.transport = new SockJS(base + (resume ? "/o=" : "/n=") + this.id, null, options);
      socket.onopen = () => {
        if (socket !== this.transport || this.readyState >= 2) return;
        if (resume) {
          socket.send("CONTINUE " + this.received.toString(16).toUpperCase());
        } else {
          this.connected = true;
          this.readyState = 1;
          this.sendPacket("0|o|");
          this.emit("open", new Event("open"));
        }
      };
      socket.onmessage = event => {
        if (socket !== this.transport || this.readyState >= 2) return;
        try { this.receive(event.data); }
        catch (_) { this.finish(4704, "Session protocol error", true); }
      };
      socket.onclose = event => {
        if (socket !== this.transport || this.readyState >= 2) return;
        if ((event.code >= 4500 && event.code < 4600) || event.code >= 4700) {
          this.finish(event.code, event.reason, true); return;
        }
        if (!this.deadline) this.deadline = Date.now() + 15000;
        this.notice(true);
        this.timer = setTimeout(() => {
          if (Date.now() >= this.deadline) this.finish(4705, "Session recovery timed out", false);
          else this.connect(this.connected);
        }, 250);
      };
    }
    discard(next) {
      const first = this.pending.keys().next().value ?? this.sent;
      if (!Number.isSafeInteger(next) || next < first || next > this.sent) throw new Error("Invalid acknowledgment");
      for (const [id, message] of this.pending) {
        if (id >= next) break;
        this.pending.delete(id);
        this.bufferedAmount -= message.bytes;
      }
    }
    receive(message) {
      let match;
      if ((match = /^(ACK|CONTINUE) ([0-9A-F]+)$/.exec(message))) {
        if (match[1] === "CONTINUE" && !this.replaying) throw new Error("Unexpected CONTINUE");
        this.discard(parseInt(match[2], 16));
        if (match[1] === "CONTINUE") {
          for (const value of this.pending.values()) this.transport.send(value.text);
          this.replaying = false;
          this.deadline = 0;
          this.notice(false);
          this.emit("reconnected", new Event("reconnected"));
        }
        return;
      }
      match = /^([0-9A-F]+)#([\s\S]*)$/.exec(message);
      if (!match) throw new Error("Missing sequence");
      const id = parseInt(match[1], 16);
      if (id < this.received) return;
      if (id !== this.received || !Number.isSafeInteger(id)) throw new Error("Sequence gap");
      this.received++;
      this.transport.send("ACK " + this.received.toString(16).toUpperCase());
      const channel = /^0\|(m|c)\|([\s\S]*)$/.exec(match[2]);
      if (!channel) throw new Error("Unexpected channel");
      if (channel[1] === "m") this.emit("message", new MessageEvent("message", {data: channel[2]}));
      else {
        const event = JSON.parse(channel[2]);
        this.finish(event.code || 4503, event.reason || "Application session ended", true);
      }
    }
    sendPacket(data) {
      const value = this.sent.toString(16).toUpperCase() + "#" + data;
      const bytes = utf8Bytes(value);
      if (this.bufferedAmount + bytes > bufferBytes || this.pending.size >= bufferMessages) {
        this.finish(4704, "Session send buffer is full", false); return;
      }
      this.pending.set(this.sent++, {text: value, bytes});
      this.bufferedAmount += bytes;
      if (this.transport.readyState === 1 && !this.replaying) this.transport.send(value);
    }
    send(data) {
      if (this.readyState !== 1) throw new Error("Socket is not open");
      this.sendPacket("0|m|" + data);
    }
    close(code = 1000, reason = "") {
      if (this.readyState >= 2) return;
      if (this.readyState === 1) this.sendPacket("0|c|" + JSON.stringify({code, reason}));
      this.finish(code, reason, true);
    }
    finish(code, reason, wasClean) {
      if (this.readyState >= 2) return;
      this.readyState = 3;
      clearTimeout(this.timer);
      this.notice(false);
      this.transport.close();
      this.pending.clear();
      this.bufferedAmount = 0;
      this.emit("close", new CloseEvent("close", {code, reason, wasClean}));
    }
  }
  function install() {
    if (!window.Shiny || !window.SockJS) return;
    // Respect application-specific socket factories.
    if (window.Shiny.createSocket) return;
    window.Shiny.createSocket = () => script.dataset.reconnect === "false"
      ? new window.SockJS(base, null, options) : new RecoveringSocket();
  }
  install();
  document.addEventListener("DOMContentLoaded", install, {once: true});
})();
