# Repository instructions

Keep shared configuration independent of machine identities and private runtime state.
Use English for source prose and Conventional Commits for commit and pull request titles.

## Verification

Run `mise x -- just check` before committing or pushing.
The check uses the pinned optional `checks` profile for configuration validation, Rust checks, workflow lint, dependency security, and publication scanning.
Add behavior to the maintained Rust helper instead of shell scripts or long just recipes.
Use synthetic credentials for tests.

## Deployment boundary

Planning and validation must preserve the running host.
Applying profiles, writing images, changing tailnet settings, and deploying services require explicit authorization.
Finish the isolated monitoring rehearsal before changes to running monitoring services.
Keep local credentials, deployment configuration, generated images, and backups in ignored local storage.
