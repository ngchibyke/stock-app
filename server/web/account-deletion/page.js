// Sign in, then ask to delete the account, or change your mind. Everything goes to this
// same server. The password is held in this page's memory only until it is used, never
// stored, and the sign-in ends when the page is done.
(function () {
  "use strict";
  var $ = function (id) { return document.getElementById(id); };
  var token = null;
  var password = null;

  var BLOCKERS = {
    open_order: "An order is still open. Wait for it to finish or cancel it.",
    open_dispute: "A report or return is still open.",
    owns_store: "You own a store. Close it first.",
    money_in_flight: "A payout or refund is still on its way. Try again when it lands."
  };

  function say(text, bad) {
    var m = $("message");
    m.textContent = text || "";
    m.className = bad ? "bad" : "";
  }

  function call(method, path, body) {
    var headers = { "Content-Type": "application/json" };
    if (token) headers["Authorization"] = "Bearer " + token;
    return fetch(path, {
      method: method,
      headers: headers,
      body: body ? JSON.stringify(body) : undefined,
      credentials: "omit",
      cache: "no-store"
    }).then(function (res) {
      return res.text().then(function (t) {
        var json = null;
        try { json = t ? JSON.parse(t) : null; } catch (e) { json = null; }
        return { status: res.status, json: json };
      });
    });
  }

  function code(r) {
    return r.json && r.json.error && r.json.error.code;
  }

  function plainError(r) {
    if (r.status === 0) return "No connection. Check your network and try again.";
    var c = code(r);
    if (c === "invalid_credentials") return "That phone number or password is not right.";
    if (c === "phone_unverified") return "Finish verifying your phone number in the app first.";
    if (r.status === 429) return "Too many tries. Wait a few minutes and try again.";
    if (r.status >= 500) return "Something went wrong on our side. Try again in a moment.";
    return "That did not work. Try again.";
  }

  function dayOf(iso) {
    var d = new Date(iso);
    if (isNaN(d.getTime())) return null;
    return d.toLocaleDateString("en-GB", { day: "numeric", month: "long", year: "numeric" });
  }

  function show(which) {
    $("signin").parentNode.hidden = which !== "signin";
    $("decide").hidden = which !== "decide";
    $("pending").hidden = which !== "pending";
  }

  function showStatus() {
    return call("GET", "/me/deletion").then(function (r) {
      if (r.status !== 200 || !r.json) { say(plainError(r), true); return; }
      if (r.json.state === "pending") {
        var day = dayOf(r.json.due_at);
        $("pending-text").textContent = day
          ? "It will be deleted on " + day + ". Until then you cannot use the app."
          : "It will be deleted after 30 days. Until then you cannot use the app.";
        show("pending");
      } else {
        $("decide-text").textContent = "Signed in. Deleting your account cannot be undone after 30 days.";
        $("blockers").textContent = "";
        show("decide");
      }
    });
  }

  function finish() {
    password = null;
    $("password").value = "";
    if (token) call("POST", "/auth/sign-out");
    token = null;
  }

  $("signin").addEventListener("submit", function (e) {
    e.preventDefault();
    var phone = $("phone").value.trim();
    var pw = $("password").value;
    if (!phone || !pw) { say("Enter your phone number and password.", true); return; }
    say("Signing in...");
    call("POST", "/auth/sign-in", { phone: phone, password: pw })
      .then(function (r) {
        if (r.status !== 200 || !r.json || !r.json.token) { say(plainError(r), true); return null; }
        token = r.json.token;
        password = pw;
        $("password").value = "";
        say("");
        return showStatus();
      })
      .catch(function () { say(plainError({ status: 0 }), true); });
  });

  $("delete").addEventListener("click", function () {
    if (!$("confirm").checked) { say("Tick the box to confirm.", true); return; }
    $("delete").disabled = true;
    say("Working...");
    call("POST", "/me/deletion", { password: password })
      .then(function (r) {
        $("delete").disabled = false;
        if (r.status === 409 && code(r) === "deletion_blocked") {
          var list = $("blockers");
          list.textContent = "";
          var reasons = (r.json.error && r.json.error.reasons) || [];
          reasons.forEach(function (c) {
            var li = document.createElement("li");
            li.textContent = BLOCKERS[c] || "Something is still open on your account.";
            list.appendChild(li);
          });
          say("We cannot delete your account yet. Finish these first:", true);
          return;
        }
        if (r.status !== 200 || !r.json) { say(plainError(r), true); return; }
        var day = r.json.due_at && dayOf(r.json.due_at);
        $("pending-text").textContent = day
          ? "It will be deleted on " + day + ". Until then you cannot use the app."
          : "It will be deleted after 30 days. Until then you cannot use the app.";
        show("pending");
        say("Your account is scheduled for deletion.");
        password = null;
      })
      .catch(function () { $("delete").disabled = false; say(plainError({ status: 0 }), true); });
  });

  $("keep").addEventListener("click", function () {
    $("keep").disabled = true;
    call("DELETE", "/me/deletion")
      .then(function (r) {
        $("keep").disabled = false;
        if (r.status !== 204) { say(plainError(r), true); return; }
        say("Your account is kept. You can use the app again.");
        show("signin");
        finish();
      })
      .catch(function () { $("keep").disabled = false; say(plainError({ status: 0 }), true); });
  });

  window.addEventListener("pagehide", finish);
  show("signin");
})();
