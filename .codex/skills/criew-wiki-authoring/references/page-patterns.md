# CRIEW Wiki Page Patterns

## Home Page

Use this when editing `Home.md`.

```md
# CRIEW Wiki

Briefly state what the wiki covers and who should read it.

## Start here

- [Install and setup](Install-and-Setup.md)
- [Configuration](Configuration.md)
- [Patch workflow](Patch-Workflow.md)
- [Troubleshooting](Troubleshooting.md)

## Topic map

### User workflows

- [Sync mail](Sync-Mail.md)
- [Review and reply](Review-and-Reply.md)
- [Apply patches](Apply-Patches.md)

### Reference

- [Configuration reference](Configuration-Reference.md)
- [Key concepts](Key-Concepts.md)

### Development

- [CRIEW repository](https://github.com/ChenMiaoi/CRIEW)
- [Contributor notes](Contributor-Notes.md)
```

## Workflow Page

Use this for task-oriented pages.

```md
# Page Title

State the task, operator, and expected outcome.

## Prerequisites

- Required environment, files, or state.

## Workflow

1. Run the first command or enter the first screen.
2. Describe the expected result.
3. Continue with the next observable step.

## Verify the result

- State what success looks like.

## Troubleshooting

- Common failure mode and the next action.

## See also

- [Related page](Related-Page.md)
```

## Reference Page

Use this for concepts, settings, or stable behavior.

```md
# Page Title

State the scope and the object being described.

## Overview

Brief definition or behavioral summary.

## Fields or options

- `key_name`: State meaning, accepted values, and important default behavior.

## Constraints

- State limits, invariants, or caveats.

## Related behavior

- Explain how this topic affects adjacent workflows.
```

## Troubleshooting Page

Use this for failure-driven guidance.

```md
# Page Title

State the symptom or failure class this page covers.

## Symptoms

- Observable error, UI state, or command output.

## Likely causes

- Most common cause first.

## Recovery steps

1. Verify the suspected cause.
2. Apply the fix.
3. Confirm recovery.

## Escalation

- State when the user should stop and inspect code, logs, or configuration in more detail.
```
