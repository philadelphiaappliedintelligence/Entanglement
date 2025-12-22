# Third Security & Logic Audit Report
## Entanglement Codebase - Sticky IDs & WebSocket Real-Time Sync

**Audit Date:** 2025-12-22  
**Focus Areas:** Sticky IDs, WebSocket Real-Time Sync, Race Conditions, Database Integrity  
**Auditor Role:** Senior Security Engineer

---

## Executive Summary

This audit examined the newly implemented **Sticky ID** system for virtual folder persistence and the **WebSocket real-time sync** mechanism. The implementation is **generally sound** with properly parameterized SQL queries and authenticated WebSocket connections. However, several issues were identified ranging from medium to low severity.

---

## Findings

### 🟡 MEDIUM: WebSocket Authentication Bypass on Failed Auth Still Upgrades Connection

* **Component:** WebSocket (Server-Side)
* **Location:** `server/src/api/ws.rs` (lines 103-112)

* **The Risk:** When JWT authentication fails, the WebSocket handler still upgrades the connection and then immediately closes it. While the socket does nothing useful, this creates unnecessary resource allocation. An attacker could repeatedly attempt connections with invalid tokens to exhaust server resources (connection pool, file descriptors).

```rust
Err(e) => {
    warn!("WebSocket auth failed: {}", e);
    // Still upgrades the connection, just closes immediately
    ws.on_upgrade(|socket| async move {
        let _ = socket;
    })
}
```

* **Mitigation:** Return an HTTP 401 response instead of upgrading the WebSocket connection on authentication failure. Use an Axum extractor or middleware to reject unauthenticated requests *before* the upgrade.

---

### 🟡 MEDIUM: No Rate Limiting on WebSocket Broadcast Channel

* **Component:** WebSocket (Server-Side)
* **Location:** `server/src/api/ws.rs` (lines 49-51)

* **The Risk:** The `SyncHub` uses a broadcast channel with a fixed capacity of 256 messages. While the server handles `Lagged` errors gracefully (line 135-137), there's no rate limiting on how fast file change events can be generated. A malicious authenticated user could rapidly create/delete files to flood the broadcast channel, causing legitimate clients to miss notifications.

```rust
impl Default for SyncHub {
    fn default() -> Self {
        Self::new(256) // Fixed buffer
    }
}
```

* **Mitigation:** Implement server-side rate limiting on file operations per user. Consider adaptive buffer sizing or dropping older messages when the buffer is full to prioritize recent events.

---

### 🟡 MEDIUM: Client-Side WebSocket Payload Lacks Filename Sanitization

* **Component:** WebSocket (Client-Side)
* **Location:** `client/macos/Entanglement/Entanglement/Services/SyncWebSocket.swift` (lines 138-155)

* **The Risk:** The client parses the `path` field from WebSocket notifications and stores it directly in `lastSyncedFile` for UI display. While this doesn't directly corrupt the filesystem (the actual sync uses the database/FileProvider), a malicious server could send crafted paths like `../../important_file` which would display incorrectly in the menu bar, potentially confusing users about what was synced.

```swift
if notification.path != "/" {
    lastSyncedFile = notification.path  // No sanitization
    lastSyncAction = notification.action
    lastSyncTime = Date()
}
```

* **Mitigation:** Sanitize the `path` before storing/displaying: strip path traversal sequences, validate structure, and consider displaying only the basename for UI purposes.

---

### 🔵 LOW: PathCache Actor Not Invalidated on Directory Move

* **Component:** FileProvider (Client-Side)
* **Location:** `client/macos/Entanglement/EntanglementFileProvider/FileProviderEnumerator.swift` (lines 11-28)

* **The Risk:** When a directory is moved server-side, the `PathCache` retains stale mappings. While `reparentItem` updates the moved folder's entry, **child folder caches are not invalidated**. This could cause enumeration failures until the next full refresh cycle.

```swift
// Only updates the moved folder, not children
if updatedInfo.isDirectory {
    await PathCache.shared.setPath(updatedInfo.path, for: updatedInfo.id)
}
```

* **Mitigation:** Implement a `PathCache.invalidateSubtree(oldPath:)` method that removes all cached entries under the old path prefix when a directory is moved.

---

### 🔵 LOW: Sticky ID Collision Theoretically Possible

* **Component:** Sticky IDs (Backend)
* **Location:** `server/src/db/files.rs` (lines 146-174)

* **The Risk:** The `original_hash_id` is set when materializing virtual folders (BLAKE3 hash of the original path). If an attacker can predict or control path names to create a hash collision, they could potentially cause a folder lookup to resolve to the wrong folder. However, BLAKE3 is cryptographically secure, making real collisions infeasible.

* **Mitigation:** None required—the risk is theoretical. For defense-in-depth, consider adding a uniqueness constraint on `original_hash_id` in the database schema.

---

### ✅ POSITIVE: SQL Injection Prevention Confirmed

* **Component:** Sticky IDs (Backend)
* **Location:** `server/src/db/files.rs` (lines 229-244)

* **The Finding:** All SQL queries handling `original_hash_id` use properly parameterized bindings via `sqlx::query_as::<_, File>()` with `$1` placeholders and `.bind(original_hash)`. **No SQL injection vulnerabilities found.**

---

### ✅ POSITIVE: Path Traversal Validation Implemented

* **Component:** REST API (Backend)
* **Location:** `server/src/api/rest/routes.rs` (lines 1210-1237)

* **The Finding:** The `validate_path()` function properly rejects:
  - Path traversal (`..`)
  - Null bytes (`\0`)
  - Backslashes (`\`)
  - Control characters
  - Protocol-relative paths (`//`)

---

### ✅ POSITIVE: WebSocket Requires JWT Authentication

* **Component:** WebSocket (Backend)
* **Location:** `server/src/api/ws.rs` (lines 91-112)

* **The Finding:** WebSocket connections require a valid JWT token passed as a query parameter. Unauthenticated connections cannot receive file change notifications.

---

### ✅ POSITIVE: Debounced FileProvider Signals Prevent Race Conditions

* **Component:** WebSocket (Client-Side)
* **Location:** `client/macos/Entanglement/Entanglement/Services/SyncWebSocket.swift` (lines 194-214)

* **The Finding:** The client implements a 500ms debounce on `signalEnumerator` calls. This prevents rapid-fire WebSocket notifications from overwhelming Finder with refresh requests.

---

### ✅ POSITIVE: Full Ownership Model Preserved

* **Component:** Shared (Backend)
* **Location:** Various functions in `server/src/db/files.rs`

* **The Finding:** All file operations include ownership checks (`owner_id = $X OR owner_id IS NULL`). The new Sticky ID and WebSocket features do not expose data to unauthenticated users.

---

## Audit Scope Checklist

| Area | Status | Notes |
|------|--------|-------|
| Sticky ID SQL Injection | ✅ Secure | Parameterized queries throughout |
| Sticky ID Integrity (Hijacking) | ✅ Secure | BLAKE3 hashes are cryptographically secure |
| Sticky ID Fallback Logic | ✅ Secure | Falls back to virtual path scan gracefully |
| WebSocket Authentication | ✅ Secure | JWT required, but connection still upgraded on failure |
| WebSocket DoS (Server) | 🟡 Medium | No rate limiting on broadcasts |
| WebSocket DoS (Client) | ✅ Secure | Debouncing prevents Finder flood |
| WebSocket Payload Trust | 🟡 Medium | Path not sanitized before display |
| FileProvider Race Conditions | 🔵 Minor | PathCache stale entries possible |
| Ownership Model | ✅ Preserved | No regressions in access control |

---

## Recommendations Summary

| Priority | Action |
|----------|--------|
| Medium | Reject WebSocket upgrade on failed auth (return 401) |
| Medium | Implement rate limiting on file change broadcasts |
| Medium | Sanitize WebSocket notification paths before UI display |
| Low | Add `PathCache.invalidateSubtree()` for directory moves |
| Low | Add DB uniqueness constraint on `original_hash_id` |

---

## Conclusion

The Sticky ID and WebSocket implementations are **architecturally sound**. The parameterized SQL queries, JWT authentication, and debounced client signaling demonstrate good security practices. The identified issues are primarily edge cases and defense-in-depth improvements rather than critical vulnerabilities. No authentication bypasses, data corruption risks, or sync loop vulnerabilities were found.
