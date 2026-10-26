// Modal focus trap and keyboard handling.
// Loaded on every page by templates/base.html. The Tab trap applies to
// whichever [role="dialog"] is open; Escape closes it only when the dialog
// offers a close target ([data-close], .modal-cancel, .btn-cancel), so a
// destructive dialog can require an explicit choice.

function openModal(id) {
    var el = document.getElementById(id);
    el.classList.remove('hidden');
    el.setAttribute('aria-hidden', 'false');
    var focusTarget = el.querySelector('input:not([type="hidden"]), select, textarea, button');
    if (focusTarget) focusTarget.focus();
}

function closeModal(id) {
    var el = document.getElementById(id);
    el.classList.add('hidden');
    el.setAttribute('aria-hidden', 'true');
}

(function() {
  document.addEventListener('keydown', function(e) {
    var modal = document.querySelector('[role="dialog"]:not([aria-hidden="true"])');
    if (!modal) return;

    if (e.key === 'Escape') {
      var closeBtn = modal.querySelector('[data-close], .modal-cancel, .btn-cancel');
      if (closeBtn) closeBtn.click();
      return;
    }

    if (e.key === 'Tab') {
      var focusable = modal.querySelectorAll('input, select, textarea, button, a[href], [tabindex]:not([tabindex="-1"])');
      if (focusable.length === 0) return;
      var first = focusable[0];
      var last = focusable[focusable.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  });
})();


// ── In-app confirm / alert ────────────────────────────────────────────────
// window.confirm()/alert() in the desktop shell surface as GTK script dialogs
// that can be swallowed (the call then never returns and the button looks
// dead). These render inside the page instead, so every engine behaves the
// same. Both return promises.

function uiDialog(message, opts) {
    opts = opts || {};
    var icon = opts.danger ? 'text-[var(--danger-text)]' : 'text-[var(--text-primary)]';
    var overlay = document.createElement('div');
    overlay.className = 'fixed inset-0 z-[120] flex items-center justify-center';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.innerHTML =
        '<div class="absolute inset-0 bg-black/60 backdrop-blur-sm"></div>' +
        '<div class="relative card w-full max-w-sm shadow-xl">' +
          '<p class="ui-dialog-message ' + icon + ' text-sm whitespace-pre-line mb-6"></p>' +
          '<div class="flex justify-end gap-2">' +
            '<button type="button" class="btn btn-secondary ui-dialog-cancel">Cancel</button>' +
            '<button type="button" class="btn ' + (opts.danger ? 'btn-danger' : 'btn-primary') + ' ui-dialog-ok">OK</button>' +
          '</div>' +
        '</div>';
    overlay.querySelector('.ui-dialog-message').textContent = String(message == null ? '' : message);
    var cancelBtn = overlay.querySelector('.ui-dialog-cancel');
    var okBtn = overlay.querySelector('.ui-dialog-ok');
    okBtn.textContent = opts.confirmLabel || 'OK';
    cancelBtn.textContent = opts.cancelLabel || 'Cancel';
    if (opts.alertOnly) cancelBtn.remove();
    document.body.appendChild(overlay);

    return new Promise(function(resolve) {
        var done = false;
        function finish(value) {
            if (done) return;
            done = true;
            document.removeEventListener('keydown', onKey, true);
            overlay.remove();
            resolve(value);
        }
        function onKey(e) {
            if (e.key === 'Escape') { e.preventDefault(); finish(false); }
            else if (e.key === 'Enter' && document.activeElement !== cancelBtn) { e.preventDefault(); finish(true); }
        }
        okBtn.addEventListener('click', function() { finish(true); });
        if (cancelBtn.parentNode) cancelBtn.addEventListener('click', function() { finish(false); });
        overlay.querySelector('.absolute.inset-0').addEventListener('click', function() { finish(false); });
        document.addEventListener('keydown', onKey, true);
        okBtn.focus();
    });
}

function uiConfirm(message, opts) {
    opts = opts || {};
    opts.alertOnly = false;
    return uiDialog(message, opts);
}

function uiAlert(message, opts) {
    opts = opts || {};
    opts.alertOnly = true;
    opts.confirmLabel = opts.confirmLabel || 'Close';
    return uiDialog(message, opts);
}

// Exposed for feature detection from inline handlers and the desktop bridge.
window.uiConfirm = uiConfirm;
window.uiAlert = uiAlert;
