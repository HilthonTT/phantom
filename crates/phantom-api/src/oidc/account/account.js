"use strict";

// Render `<time data-ts="unix-seconds">` in the viewer's locale and timezone.
document.querySelectorAll("time[data-ts]").forEach((el) => {
  const ts = Number(el.dataset.ts);
  if (!ts) {
    return;
  }

  const date = new Date(ts * 1000);
  el.dateTime = date.toISOString();
  el.title = date.toISOString();
  el.textContent = date.toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
});

// Forms with `data-confirm="..."` ask before submitting; all forms disable
// their submit buttons once sent so a double click can't post twice.
document.querySelectorAll("form").forEach((form) => {
  form.addEventListener("submit", (event) => {
    const message = form.dataset.confirm;
    if (message && !window.confirm(message)) {
      event.preventDefault();
      return;
    }

    form.querySelectorAll("button[type=submit], button:not([type])").forEach((button) => {
      button.disabled = true;
    });
  });
});
