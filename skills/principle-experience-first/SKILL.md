---
name: principle-experience-first
description: "Apply when product, UX, or feature-scope tradeoffs come up. Choose user delight over implementation convenience; ship fewer polished features over more rough ones."
---

# Experience First

## External action boundary

Local, reversible work inside the user-scoped workspace may proceed when the request authorizes the task. Before sending any external message or performing any state-changing action in an external service, obtain explicit user authorization for that exact class of action. This includes posting comments, opening or merging pull requests, updating tickets, starting deployments or remote jobs, sending email or chat messages, and changing hosted settings. Read-only external inspection is allowed when needed. A general autonomy instruction does not replace this authorization.

When implementation convenience conflicts with user delight, choose delight.

- Every feature, control, and option must be justified
- Ship less, ship better (polished experience with three features beats rough one with ten)
- Prototype before committing (design decisions are cheaper in throwaway HTML than production code)
- Get the details right (transitions, alignment, spacing, feedback, error states)
- Tighten the core loop (every feature should serve the central workflow or get out of the way)

The user is whoever consumes the work. For a UI that is the end user. For a library or an internal API it is the colleague who imports it. The engineer who maintains the code next is a user too. Weigh their experience the same way, and explain impact from their perspective.

Foundations should serve the experience. Foundational thinking governs the *sequence* of work. This principle governs the *target*.
