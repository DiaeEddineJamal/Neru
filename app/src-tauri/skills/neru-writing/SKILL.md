---
name: neru-writing
description: Use for any prose a person will read - replies, summaries, READMEs, docs, commit messages, UI copy, error messages, emails. Keeps writing plain, specific and human instead of generic AI-sounding text.
---

# Writing that does not read like AI

Readers skim. They trust text that is specific and quiet, and they stop trusting text that sounds
like a brochure. Write the way a careful senior engineer writes to a colleague.

## Before you write

- Know the one thing the reader must take away. Put it first.
- Know who reads it: the user in chat, a teammate in a README, a stranger in an error message.
- If it does not help the reader act or understand, leave it out.

## Rules

1. **Lead with the answer or the result.** "The build fails because `vite` is missing" beats a
   paragraph of context before it.
2. **Be concrete.** Name the file, the command, the number, the version. Replace "various issues"
   with the actual issues. Replace "improved performance" with what got faster and by how much.
3. **Plain words.** Use, not utilize. Help, not facilitate. Start, not commence. Now, not at this
   point in time.
4. **Short sentences, active voice, real subjects.** "Neru writes the file" not "The file is
   written".
5. **Say it once.** No summary of what you are about to say, no recap of what you just said.
6. **Hedge only real uncertainty, and say what it is.** "I did not run the tests" is useful.
   "This may potentially help" is noise.
7. **Formatting serves scanning.** Headings and bullets for reference material and steps; prose for
   reasoning. Do not bullet a single thought or bold every other phrase.

## Words and patterns to cut

- Openers and closers: "Great question", "Certainly!", "I hope this helps", "Let me know if you
  have any other questions", "In conclusion".
- Inflated vocabulary: delve, leverage, robust, seamless, cutting-edge, game-changer, elevate,
  unlock, empower, harness, streamline, tapestry, landscape, realm, testament, pivotal, crucial
  (when it is not), comprehensive (when it is not).
- Filler: "It's worth noting that", "It is important to", "In today's fast-paced world",
  "When it comes to", "At the end of the day".
- Formulaic structures: "It's not just X, it's Y", "Whether you're A or B", triplets of adjectives,
  every paragraph ending on an uplifting line.
- Emoji and exclamation marks in technical writing unless the user uses them.
- Em dashes used as a tic. One per page is plenty; a comma or a period usually works.

## UI copy and error messages

- Buttons say what happens: "Save changes", "Delete project", not "OK" or "Submit".
- Errors say what went wrong, why if known, and what to do next, in that order:
  "Could not reach the provider. Check your internet connection, then try again."
- No blame, no jokes, no "Oops!". No raw stack traces in the main message; put details behind a
  "Details" control.
- Empty states tell people how to fill them.

## Before you send

Read it once as the reader. Delete the first sentence if it is warm-up. Delete the last sentence
if it is a summary. Check every claim is something you actually know.
