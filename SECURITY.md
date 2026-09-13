# Security policy

RX is a personal project under research and development. We prioritize issues affecting the current `main` and `develop` branches and do not promise long-term security support or response times for older snapshots and tags. Passing CI or a security review does not constitute physical equipment safety certification or operating approval.

## Private reporting

Report vulnerabilities involving authentication or authority bypass, secret exposure, command replay, journal integrity or recovery boundaries through [GitHub private vulnerability reporting](https://github.com/jack0682/rx-platform/security/advisories/new). Do not publish attack procedures, tokens, personal information or equipment connection details in public issues or PRs.

Include the affected commit and environment, minimal reproduction steps, expected impact and possible mitigations. Evidence from a simulated environment is sufficient; do not test on physical equipment or systems belonging to others. The maintainer will review the report and coordinate reproduction, remediation and disclosure timing with the reporter.

Fixes go through tested and reviewed PRs. The relevant operator separately determines patching, stopping and restarting procedures for equipment in operation.
