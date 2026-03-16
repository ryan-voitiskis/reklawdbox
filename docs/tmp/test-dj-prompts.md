# Test & Refine DJ Prompts

You are an orchestrator agent. Your job is to test the 6 DJ prompts defined in `site/src/content/docs/workflows/dj-prompts.mdx` by running each one as a subagent against the real library, evaluating the results, and proposing refinements.

## Setup

- The reklawdbox MCP server is connected and provides all tools (`search_tracks`, `read_library`, `query_transition_candidates`, `score_transition`, `build_set`, `lookup_discogs`, `lookup_beatport`, etc.)
- The library has ~2,500 tracks across techno, house, breakbeat, ambient, and other genres
- Cache coverage should be checked first: call `cache_coverage()` and note any gaps

## Test procedure

For each of the 6 prompts, spawn a subagent with the prompt text and a realistic user scenario. The subagent should have access to all MCP tools. Evaluate the subagent's output against the criteria below.

### Test scenarios

**1. Gig Prep**

Run the prompt with this context:
```
I have a gig coming up. Here's the context:
- Venue/event: small club, dark room, 200 capacity
- Time slot: 1-3am, I'm the second of three DJs
- Duration: 2 hours
- Who's playing before/after me: opener plays deep house, closer plays hard techno
- Vibe I'm going for: hypnotic, driving but not aggressive
- Anything I definitely want to play: anything by Polar Inertia
```

Evaluate:
- Did the agent translate the context into reasonable BPM/energy/genre constraints?
- Did it account for the position between deep house and hard techno?
- Is the pool size appropriate (~30-50 tracks for a 2-hour set)?
- Did it find the Polar Inertia tracks and integrate them?
- Did it present a pool (not just a fixed sequence)?

**2. Collection Gap Analysis**

Run the prompt as-is (no customization needed).

Evaluate:
- Did the agent check BPM distribution, key coverage, genre spread, and energy range?
- Did it identify specific gaps (not just describe what exists)?
- Are the gaps actionable ("you have nothing in 12A" not just "12A has few tracks")?
- Did it check for harmonic dead ends?
- Is the analysis useful to a working DJ, or just academic?

**3. Dig Session Partner**

Run with:
```
I'm digging for new music today. Here's what I'm looking for:
Dark, atmospheric techno. Stuff that sounds like it was made in a bunker.
Around 128-134 BPM. Think Polar Inertia, SNTS, Silent Servant.

Use my collection to understand my taste and suggest directions.
```

Evaluate:
- Did the agent analyze the collection for relevant patterns (labels, related artists)?
- Did it use lookup tools to research suggestions?
- Are the suggestions specific (artist + release names) or vague ("check out dark techno labels")?
- Did it avoid suggesting what the user already owns?
- Did it provide reasoning for each suggestion?

**4. Post-Gig Debrief**

Skip this test — depends on session history tools (issue #24) which are not yet implemented. Note this in the report.

**5. Harmonic Journey Planning**

Run with:
```
Plan a harmonic journey for my next set.
- Starting key: 6A
- Style: rising tension
- Duration: 12 tracks
- Pool: my whole library
```

Evaluate:
- Did the agent map available tracks by Camelot position?
- Did it plan a coherent clockwise journey (6A → 7A → 8A → ...)?
- Did it flag any positions where the library is thin?
- Are the suggested tracks real tracks from the library (not hallucinated)?
- Did it consider BPM compatibility alongside key movement?

**6. Practice Session Design**

Run with:
```
Design a practice session for me.
Focus: learn my new tracks (the Polar Inertia and Buttechno tracks I just imported)
Duration: 45 min
```

Evaluate:
- Did the agent find the recently imported tracks?
- Did it pair new tracks with familiar ones for context?
- Are the suggested transitions achievable but non-trivial?
- Did it use `score_transition` to validate the pairings?
- Is the session structured as progressive exercises, not just a random list?

## Evaluation criteria

For each prompt, rate on these axes (1-5):

| Axis | 1 (poor) | 5 (excellent) |
|------|----------|---------------|
| **Tool use** | Ignored available tools, hallucinated data | Used the right tools, grounded all claims in real data |
| **Context translation** | Ignored the user's context, gave generic output | Deeply incorporated context into every decision |
| **Actionability** | Vague suggestions, "you could try..." | Specific tracks, specific reasons, ready to use |
| **DJ knowledge** | Technically correct but musically naive | Shows understanding of how DJs actually work |
| **Conversation quality** | Robotic, dumped everything at once | Natural flow, asked clarifying questions, invited iteration |

## Output

For each prompt, produce:
1. The subagent's full output (or a representative summary if very long)
2. Scores on the 5 axes
3. Specific issues found (e.g., "hallucinated an artist not in the library", "suggested a pool of 200 tracks which is too large", "didn't ask about the DJ's comfort with key jumps")
4. Proposed refinements to the prompt's "What the agent should do" section — specific wording changes with reasoning

After testing all prompts, produce a summary with:
- Overall patterns (which axes are consistently weak?)
- Cross-prompt issues (e.g., "none of them ask clarifying questions before diving in")
- Priority-ordered list of refinements
- A diff-ready set of changes to `dj-prompts.mdx`
