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

// Self-contained styles: client.html loads no stylesheet, so the dialog must
// not depend on Tailwind utilities, .card or .btn. Custom properties fall
// back to the dark palette when a page does not define them.
var UI_DIALOG_CSS = [
  '.ui-dialog-overlay{position:fixed;top:0;right:0;bottom:0;left:0;z-index:2147483000;',
  'display:flex;align-items:center;justify-content:center;padding:20px;',
  'background:rgba(2,6,23,.62);font-family:inherit;margin:0}',
  '.ui-dialog-card{box-sizing:border-box;width:min(440px,100%);max-height:calc(100vh - 40px);',
  'overflow:auto;padding:22px 22px 18px;border-radius:12px;',
  'background:var(--bg-card,#1e293b);color:var(--text-primary,#e2e8f0);',
  'border:1px solid var(--border,#334155);',
  'box-shadow:0 24px 60px rgba(0,0,0,.5)}',
  '.ui-dialog-message{margin:0 0 20px;font-size:14px;line-height:1.5;white-space:pre-wrap;',
  'color:var(--text-primary,#e2e8f0);word-break:break-word}',
  '.ui-dialog-message.ui-dialog-danger{color:var(--danger-text,#fca5a5)}',
  '.ui-dialog-actions{display:flex;gap:10px;justify-content:flex-end;flex-wrap:wrap}',
  '.ui-dialog-btn{appearance:none;-webkit-appearance:none;font:inherit;font-size:13px;',
  'font-weight:600;line-height:1;padding:10px 16px;border-radius:8px;cursor:pointer;',
  'border:1px solid var(--border,#334155);',
  'background:var(--bg-hover,#334155);color:var(--text-primary,#e2e8f0)}',
  '.ui-dialog-btn:hover{filter:brightness(1.12)}',
  '.ui-dialog-btn:focus-visible{outline:2px solid var(--accent,#60a5fa);outline-offset:2px}',
  '.ui-dialog-btn:disabled{opacity:.45;cursor:not-allowed;filter:none}',
  '.ui-dialog-field{margin:0 0 18px}',
  '.ui-dialog-field-label{display:block;font-size:12.5px;line-height:1.4;margin-bottom:6px;',
  'color:var(--text-muted,#94a3b8);word-break:break-word}',
  '.ui-dialog-input{box-sizing:border-box;width:100%;padding:9px 11px;font:inherit;font-size:13.5px;',
  'color:var(--text-primary,#e2e8f0);background:var(--bg-input,#0f172a);',
  'border:1px solid var(--border,#334155);border-radius:7px}',
  '.ui-dialog-input:focus{outline:none;border-color:var(--border-focus,#3b82f6);',
  'box-shadow:0 0 0 2px rgba(59,130,246,.25)}',
  '.ui-dialog-btn-ok{background:var(--accent,#2563eb);border-color:var(--accent,#2563eb);',
  'color:var(--accent-contrast,#f8fafc)}',
  '.ui-dialog-btn-danger{background:var(--danger,#b91c1c);border-color:var(--danger,#b91c1c);',
  'color:#fff}'
].join('');

function uiDialogStyles() {
    if (document.getElementById('ui-dialog-style')) return;
    var style = document.createElement('style');
    style.id = 'ui-dialog-style';
    style.textContent = UI_DIALOG_CSS;
    (document.head || document.documentElement).appendChild(style);
}

function uiDialog(message, opts) {
    opts = opts || {};
    uiDialogStyles();
    // One dialog at a time: a stray overlay from an earlier click (or a page
    // that never resolved it) must not stack up behind the new one.
    Array.prototype.forEach.call(document.querySelectorAll('.ui-dialog-overlay'), function(el) {
        el.parentNode.removeChild(el);
    });

    var overlay = document.createElement('div');
    overlay.className = 'ui-dialog-overlay';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.setAttribute('aria-live', 'assertive');

    var card = document.createElement('div');
    card.className = 'ui-dialog-card';

    var text = document.createElement('p');
    text.className = 'ui-dialog-message' + (opts.danger ? ' ui-dialog-danger' : '');
    text.textContent = String(message == null ? '' : message);
    card.appendChild(text);

    var requireText = typeof opts.requireText === 'string' ? opts.requireText : '';
    var confirmInput = null;
    if (requireText && !opts.alertOnly) {
        var field = document.createElement('div');
        field.className = 'ui-dialog-field';
        var label = document.createElement('label');
        label.className = 'ui-dialog-field-label';
        label.htmlFor = 'ui-dialog-confirm-input';
        label.textContent = 'Type ' + requireText + ' to confirm';
        confirmInput = document.createElement('input');
        confirmInput.type = 'text';
        confirmInput.id = 'ui-dialog-confirm-input';
        confirmInput.className = 'ui-dialog-input';
        confirmInput.autocomplete = 'off';
        confirmInput.spellcheck = false;
        field.appendChild(label);
        field.appendChild(confirmInput);
        card.appendChild(field);
    }

    var actions = document.createElement('div');
    actions.className = 'ui-dialog-actions';

    var cancelBtn = document.createElement('button');
    cancelBtn.type = 'button';
    cancelBtn.className = 'ui-dialog-btn ui-dialog-cancel';
    cancelBtn.textContent = opts.cancelLabel || 'Cancel';

    var okBtn = document.createElement('button');
    okBtn.type = 'button';
    okBtn.className = 'ui-dialog-btn ui-dialog-ok' + (opts.danger ? ' ui-dialog-btn-danger' : ' ui-dialog-btn-ok');
    okBtn.textContent = opts.confirmLabel || 'OK';

    if (!opts.alertOnly) actions.appendChild(cancelBtn);
    actions.appendChild(okBtn);
    if (confirmInput) {
        okBtn.disabled = true;
        confirmInput.addEventListener('input', function() {
            okBtn.disabled = confirmInput.value.trim() !== requireText;
        });
    }
    card.appendChild(actions);
    overlay.appendChild(card);
    document.body.appendChild(overlay);

    var previousFocus = document.activeElement;

    return new Promise(function(resolve) {
        var done = false;
        function finish(value) {
            if (done) return;
            done = true;
            document.removeEventListener('keydown', onKey, true);
            if (overlay.parentNode) overlay.parentNode.removeChild(overlay);
            if (previousFocus && previousFocus.focus) {
                try { previousFocus.focus(); } catch (e) {}
            }
            resolve(value);
        }
        function onKey(e) {
            if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finish(false); }
            else if (e.key === 'Enter' && document.activeElement !== cancelBtn) {
                e.preventDefault(); e.stopPropagation();
                if (!okBtn.disabled) finish(true);
            }
        }
        okBtn.addEventListener('click', function(e) { e.stopPropagation(); finish(true); });
        cancelBtn.addEventListener('click', function(e) { e.stopPropagation(); finish(false); });
        overlay.addEventListener('mousedown', function(e) {
            if (e.target === overlay) finish(false);
        });
        overlay.addEventListener('keydown', function(e) { e.stopPropagation(); });
        document.addEventListener('keydown', onKey, true);
        if (confirmInput) {
            // Enter inside the field submits only when the text matches;
            // the document-level handler ignores a disabled confirm.
            confirmInput.addEventListener('keydown', function(e) {
                if (e.key !== 'Enter') return;
                e.preventDefault();
                e.stopPropagation();
                if (!okBtn.disabled) finish(true);
            });
            confirmInput.focus();
        } else {
            okBtn.focus();
        }
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
