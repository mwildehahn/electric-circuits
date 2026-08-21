# The change log is rotated into segments by the engine; a segment is deleted once nothing can resume inside it

Status: accepted (2026-08-21)

Since the sequencer architecture, every committed change to every tracked table is appended to one
global `changes` stream forever; the durable-streams server offers whole-stream TTL but no prefix
trimming, so a long-running deployment fills its disk. We rotate engine-side. At a transaction
boundary, by bytes or age, the ingestor appends a final `{control: "rotated", next: "changes/<n+1>"}`
envelope to segment *n*, **closes** it, and continues in *n+1*; the sequencer's live read is released
with `stream-closed` and follows the pointer, and a dormant shape's resume offset becomes
`(segment, offset)` and replays across the pointer the same way. A segment is deleted when the
sequencer's checkpoint is past it and no dormant shape resumes inside it; a dormant shape that would
pin a segment beyond the retention window is evicted first. The catalog records `ChangesRotated`.

## Considered options

- Prefix trimming as a durable-streams primitive (upstream issue #12's plan): cleaner offsets, but a
  change to another upstream. The low-watermark computation is the same, so it can replace rotation
  later without touching the policy.
