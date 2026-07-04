// Declarative Jenkins pipeline for the hpc workspace.
//
// Stages: Checkout → Toolchain → Lint → Build → Test → Bench → Package.
// Build artifacts (release binaries + benchmark output) are archived, and the
// final status is reported back to GitHub via the GitHub plugin's githubNotify
// step. Mirrors .github/workflows/ci.yml so both CI systems enforce the same
// gate.
//
// Requirements on the agent: a C toolchain, protobuf-compiler, libclang and
// network access to install the Rust toolchain via rustup. The githubNotify
// steps require the "GitHub" Jenkins plugin.

pipeline {
    agent any

    options {
        timestamps()
        timeout(time: 45, unit: 'MINUTES')
        disableConcurrentBuilds()
    }

    environment {
        CARGO_TERM_COLOR = 'always'
        // Keep cargo state inside the workspace so it is cleaned with the job.
        CARGO_HOME = "${WORKSPACE}/.cargo"
        PATH = "${WORKSPACE}/.cargo/bin:${PATH}"
    }

    stages {
        stage('Checkout') {
            steps {
                checkout scm
                githubNotify context: 'jenkins/pipeline', status: 'PENDING', description: 'Build started'
            }
        }

        stage('Toolchain') {
            steps {
                sh '''
                    set -euo pipefail
                    if ! command -v cargo >/dev/null 2>&1; then
                        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
                            | sh -s -- -y --profile minimal --default-toolchain stable
                    fi
                    rustup component add clippy rustfmt
                    rustc --version
                    cargo --version
                '''
            }
        }

        stage('Lint') {
            steps {
                sh 'cargo fmt --all --check'
                sh 'cargo clippy --workspace --all-targets -- -D warnings'
            }
        }

        stage('Build') {
            steps {
                sh 'cargo build --workspace --release'
            }
        }

        stage('Test') {
            steps {
                sh 'cargo test --workspace'
            }
        }

        stage('Bench') {
            steps {
                sh 'cargo bench -p hpc-bench -- --output-format bencher | tee bench-output.txt'
            }
        }

        stage('Package') {
            steps {
                sh '''
                    set -euo pipefail
                    mkdir -p dist
                    for bin in hpc hpc-daemon hpc-agent hpc-monitor hpc-diag; do
                        if [ -x "target/release/${bin}" ]; then
                            cp "target/release/${bin}" dist/
                        fi
                    done
                    tar -czf "hpc-dist-${BUILD_NUMBER}.tar.gz" -C dist .
                '''
            }
        }
    }

    post {
        success {
            archiveArtifacts artifacts: 'hpc-dist-*.tar.gz, bench-output.txt', fingerprint: true, allowEmptyArchive: true
            githubNotify context: 'jenkins/pipeline', status: 'SUCCESS', description: 'Build passed'
        }
        failure {
            githubNotify context: 'jenkins/pipeline', status: 'FAILURE', description: 'Build failed'
        }
        always {
            echo "Pipeline finished with result: ${currentBuild.currentResult}"
        }
    }
}
