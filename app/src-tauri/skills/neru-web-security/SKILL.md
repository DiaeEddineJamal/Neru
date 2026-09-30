---
name: neru-web-security
description: Use when building or reviewing a web app, API or backend for security - auth, sessions, input handling, secrets, headers, uploads, dependencies - or when the user asks for a security review. OWASP-based checklist with concrete fixes.
---

# Web application security review

Security problems hide in the boring parts: defaults, glue code, error paths. Review those first,
then the clever code. Report findings with a file and line, what an attacker can do, and the fix.

## How to work

1. **Map the surface.** List entry points (routes, handlers, CLI args, message consumers, file
   uploads, webhooks) and where trust changes (browser to server, service to service, user to
   admin). Use search_text and find_files; read the routing and middleware setup.
2. **Follow untrusted data** from each entry point to where it is used: database queries, shell
   commands, file paths, HTML output, redirects, deserialisers, outbound requests.
3. **Check the defaults**, because insecure defaults ship to production (see below).
4. **Rank findings** by impact and likelihood: Critical (remote code execution, auth bypass, data
   exposure across users), High, Medium, Low. Say plainly when something is only a hardening idea.
5. **Fix or propose the fix** with the smallest correct change, and a test when practical.

## Checklist

**Injection**
- SQL/NoSQL: parameterised queries or the ORM's bound parameters; never string-built queries.
- Commands: no shell with user input; pass argument arrays; allowlist values.
- Paths: resolve and confirm the result stays inside the intended root (`..`, absolute paths,
  symlinks, Windows drive letters and `\\?\`).
- Templates/HTML: auto-escaping on; no `dangerouslySetInnerHTML`/`v-html`/`innerHTML` with user data
  unless sanitised (DOMPurify).
- Server-side request forgery: outbound fetches to user URLs are allowlisted and block private,
  loopback and metadata addresses (169.254.169.254), also after redirects and DNS resolution.

**Authentication and sessions**
- Passwords hashed with Argon2id, scrypt or bcrypt; never MD5/SHA-1/plain SHA-256.
- Sessions: `HttpOnly`, `Secure`, `SameSite=Lax` or `Strict`; rotate the id on login; expire them.
- JWTs: verify signature and `alg` (reject `none`), `exp`, audience; do not keep secrets in them;
  short lifetimes with refresh.
- Rate-limit login, password reset and OTP endpoints; lock out or slow down on repeated failure.
- Password reset tokens are single-use, random (at least 128 bits), and expire.

**Authorisation**
- Every object lookup checks the caller may access that object (insecure direct object references:
  `/api/orders/123` must check ownership), including on update and delete.
- Admin actions checked on the server, never only hidden in the UI.
- Deny by default; one central check rather than scattered `if` statements.

**Secrets and configuration (insecure defaults)**
- No secrets, keys or tokens in the repository, client bundle, logs or error messages. Use
  environment variables or a secret manager; check `.env` is gitignored.
- No default or example credentials that work in production; no `admin/admin`.
- Debug modes, stack traces, GraphQL introspection, Swagger UIs and verbose errors off in production.
- CORS: never `*` with credentials; list exact origins.
- Fail closed: if a permission check, signature check or config value is missing, deny.

**Browser protections**
- `Content-Security-Policy` without `unsafe-inline` for scripts where possible;
  `Strict-Transport-Security`; `X-Content-Type-Options: nosniff`; `Referrer-Policy`;
  `frame-ancestors` (or `X-Frame-Options`) to prevent clickjacking.
- CSRF protection for cookie-authenticated state changes (SameSite plus tokens for sensitive forms).
- `rel="noopener noreferrer"` on links opening new windows to untrusted sites.

**Files and uploads**
- Check type by content, not only extension; limit size; store outside the web root with random
  names; never execute or serve uploads as HTML from the app's origin.
- Archive extraction guards against zip-slip and zip bombs.

**Data**
- Encrypt in transit (TLS) and, for sensitive data, at rest. Minimise what you collect and log.
- Logs must not contain passwords, tokens, full card numbers or personal data.

**Dependencies**
- Lockfile committed; audit with the ecosystem's tool (`npm audit`, `pip-audit`, `cargo audit`)
  and read advisories that match installed versions. Prefer maintained, widely used packages;
  question tiny packages that do trivial things.

## Reporting

For each finding: severity, `path:line`, what an attacker can do in one sentence, the fix, and how
to verify it. Lead with the most severe. If you found nothing serious, say what you checked so the
user knows the coverage.
