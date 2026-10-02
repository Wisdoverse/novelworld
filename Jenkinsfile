// Optional source-based deployment from a dedicated trusted job on its Linux server.
// Keep the Job's default workspace persistent on that host and Docker daemon.
pipeline {
    agent any

    options {
        skipDefaultCheckout(true)
        disableConcurrentBuilds()
        timestamps()
        timeout(time: 60, unit: 'MINUTES')
        buildDiscarder(logRotator(numToKeepStr: '20'))
    }

    environment {
        // The same project must own the data volumes on every run.
        COMPOSE_PROJECT_NAME = 'novelworld'
    }

    stages {
        stage('Checkout') {
            steps {
                script {
                    if (env.CHANGE_ID) {
                        error('Use a trusted branch job on the server agent; pull requests belong in CI.')
                    }
                }
                // Check before SCM can replace a managed release checkout.
                sh '''#!/usr/bin/env bash
set -euo pipefail
if [[ -e .release ]]; then
    printf 'Jenkins source deployment cannot replace managed release state; use the DEPLOY.md release procedure.\n' >&2
    exit 1
fi
'''
                checkout scm
            }
        }

        stage('Build server images') {
            steps {
                // The example file supplies Compose interpolation only; no containers start.
                sh '''#!/usr/bin/env bash
set -euo pipefail
docker compose --env-file .env.example build \
    gateway user-service novel-service agent-service narrative-service frontend
'''
            }
        }

        stage('Deploy server') {
            steps {
                sh '''#!/usr/bin/env bash
set -euo pipefail
[[ ! -e .release ]] || {
    printf 'Managed release state appeared during the build; source deployment refused.\n' >&2
    exit 1
}
[[ -f .env ]] || {
    printf 'Preconfigure the persistent server .env as described in DEPLOY.md before the first build.\n' >&2
    exit 1
}
# Keep stdin closed so incomplete database setup fails instead of prompting.
# The launcher drains old writers, builds, migrates, and waits for readiness.
bash start.sh </dev/null
bash infra/ops/health-checks.sh
'''
            }
        }
    }
}
