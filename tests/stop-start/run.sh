#!/bin/sh
# `meridian stop` and `meridian start` on a real cluster: k3d, made here and
# deleted after, never a shared deployment. The release is a stand-in for
# meridian-runtime's workloads (tests/stop-start/chart), installed with helm
# as `meridian up` would leave one: components as Deployments, a launched
# plugin, the database's StatefulSet with its volume, the broker's, and the
# release's CronJobs.
#
# What it holds the commands to: the round trip, each count and each CronJob's
# schedule given back exactly, nothing deleted; each refusal, with nothing
# changed; a stop and a start cut short, each finished by running it again;
# `status` and `status --json` while stopped; `doctor` while stopped; and the
# commands that would start it again refused, pointing to `meridian start`.
#
# Run by `make e2e-stop-start`, in the image its Dockerfile.rust stage builds,
# on the host's network and with the host's docker daemon.
set -eu

CLUSTER="${STOP_START_CLUSTER:-meridian-stop-start}"
NS=meridian
DB=meridian-meridian-runtime-database
CHART=/w/chart

failed() {
	echo "stop-start FAILED: $*" >&2
	exit 1
}

step() { echo "stop-start: $*"; }

cleanup() { k3d cluster delete "$CLUSTER" >/dev/null 2>&1 || true; }
trap cleanup EXIT INT TERM

cleanup
step "a cluster of its own, $CLUSTER"
k3d cluster create "$CLUSTER" --no-lb --wait --timeout 180s \
	--k3s-arg '--disable=traefik@server:0' >/dev/null
kubectl wait --for=condition=Ready nodes --all --timeout=120s >/dev/null

step "the stand-in release, installed"
helm install meridian "$CHART" -n "$NS" --create-namespace --wait --timeout 180s >/dev/null

# Each Deployment and StatefulSet with its count, and each CronJob with its
# suspend, as one line each: what a round trip must give back exactly.
counts() {
	kubectl get deployments,statefulsets -n "$NS" \
		-o jsonpath='{range .items[*]}{.kind}/{.metadata.name}={.spec.replicas}{"\n"}{end}' | sort
	kubectl get cronjobs -n "$NS" \
		-o jsonpath='{range .items[*]}cronjob/{.metadata.name}={.spec.suspend}{"\n"}{end}' | sort
}
replicas() { kubectl get "$1" -n "$NS" -o jsonpath='{.spec.replicas}'; }
suspended() { kubectl get cronjob "$1" -n "$NS" -o jsonpath='{.spec.suspend}'; }
record() { kubectl get statefulset "$DB" -n "$NS" -o jsonpath='{.metadata.annotations.meridian\.dev/stopped}'; }

before="$(counts)"
claims="$(kubectl get pvc -n "$NS" -o name | sort)"
[ -n "$claims" ] || failed "the database's volume was not made"

# Runs `meridian <args>`; its exit code in $code and what it said in $said.
run() {
	set +e
	said="$(meridian "$@" 2>&1)"
	code=$?
	set -e
}

# `meridian stop` refused with $1 in what it says, and nothing changed.
refused_with() {
	run stop --yes
	[ "$code" = 1 ] || failed "stop exited $code, not 1, where it should refuse ($1): $said"
	printf '%s' "$said" | grep -qF "$1" || failed "stop's refusal does not say \"$1\": $said"
	[ "$(counts)" = "$before" ] || failed "a refused stop changed something ($1)"
	[ -z "$(record)" ] || failed "a refused stop left its record ($1)"
	step "refused: $1"
}

# ── Each refusal ─────────────────────────────────────────────────────────────

kubectl annotate statefulset "$DB" -n "$NS" \
	'meridian.dev/stopped-for-restore={"database":1,"deployments":[],"act":"restore-to-time","since":"2026-11-24T00:00:00Z"}' >/dev/null
refused_with "was stopped by \`meridian restore --to-time\`"
kubectl annotate statefulset "$DB" -n "$NS" meridian.dev/stopped-for-restore- >/dev/null

# A Job of each kind a stop never cuts short, running; then gone.
job() {
	kubectl apply -n "$NS" -f - >/dev/null <<-EOF
		apiVersion: batch/v1
		kind: Job
		metadata:
		  name: $1
		  labels: { app.kubernetes.io/instance: meridian, $2 }
		spec:
		  template:
		    spec:
		      restartPolicy: Never
		      terminationGracePeriodSeconds: 0
		      containers: [ { name: job, image: "busybox:1.36.1", command: ["sleep", "600"] } ]
	EOF
	kubectl wait -n "$NS" --for=jsonpath='{.status.active}'=1 "job/$1" --timeout=120s >/dev/null
}
for each in \
	"meridian-meridian-runtime-migrate-9|meridian.dev/component: migrate|a migration" \
	"meridian-restore-1|meridian.dev/component: restore|a restore" \
	"meridian-meridian-runtime-rollback-1|meridian.dev/component: rollback|a rollback" \
	"$DB-offline-1|meridian.dev/database-act: restore-to-time|an act on the database's volume"; do
	name="${each%%|*}"
	rest="${each#*|}"
	label="${rest%%|*}"
	what="${rest#*|}"
	job "$name" "$label"
	refused_with "job/$name, $what"
	kubectl delete job "$name" -n "$NS" --wait=true >/dev/null
done

# Helm holding the release as pending: an upgrade that waits on a component
# that is never ready, held while the stop is asked, then rolled back.
helm upgrade meridian "$CHART" -n "$NS" --set hang=true --wait --timeout 60s >/dev/null 2>&1 &
upgrading=$!
tries=0
until helm status meridian -n "$NS" 2>/dev/null | grep -q '^STATUS: pending-upgrade'; do
	tries=$((tries + 1))
	[ "$tries" -lt 60 ] || failed "the upgrade was never pending"
	sleep 1
done
before_pending="$before"
before="$(counts)"
refused_with "Helm holds it as pending-upgrade"
wait "$upgrading" || true
helm rollback meridian 1 -n "$NS" --wait --timeout 180s >/dev/null
before="$before_pending"
[ "$(counts)" = "$before" ] || failed "the rollback did not give back the release as it was"

# ── The round trip ───────────────────────────────────────────────────────────

run stop --yes
[ "$code" = 0 ] || failed "stop exited $code: $said"
printf '%s' "$said" | grep -qF "Kept running, as under a restore: meridian-meridian-runtime-broker" \
	|| failed "stop's plan does not say the broker keeps running: $said"
for deployment in $(kubectl get deployments -n "$NS" -o name); do
	[ "$(replicas "$deployment")" = 0 ] || failed "$deployment still runs"
done
[ "$(replicas "statefulset/$DB")" = 0 ] || failed "the database still runs"
[ "$(replicas statefulset/meridian-meridian-runtime-broker)" = 1 ] || failed "the broker was stopped"
[ "$(suspended meridian-meridian-runtime-backup)" = true ] || failed "the backup's schedule was not suspended"
[ "$(suspended meridian-meridian-runtime-scrub)" = true ] || failed "the scrub was given a schedule"
[ -n "$(record)" ] || failed "nothing was recorded on $DB"
[ "$(kubectl get pvc -n "$NS" -o name | sort)" = "$claims" ] || failed "a volume was deleted"
step "stopped: every component and the database at none, the broker running, the volume kept"

run status
[ "$code" = 0 ] || failed "status exited $code: $said"
printf '%s' "$said" | grep -q '^Stopped since [0-9T:Z-]*; run `meridian start`$' \
	|| failed "status does not say the one line: $said"
printf '%s' "$said" | grep -q 'Components:' && failed "status still says the components: $said"
run status --json
printf '%s' "$said" | grep -q '"stopped":{"since":"[0-9T:Z-]*"}' \
	|| failed "status --json has no stopped.since: $said"
step "status says it in one line, and --json its stopped.since"

run doctor
printf '%s' "$said" | grep -qF "  worth    meridian in meridian is stopped since" \
	|| failed "doctor does not say it is stopped, as worth knowing: $said"
# What else this cluster stops an install for (its Kubernetes, the platform
# it cannot reach) is the cluster's; nothing of what was stopped stops it.
printf '%s' "$said" | grep -A1 '^  stops' | grep -qiE 'replica|stopped|meridian-runtime' \
	&& failed "doctor fails on what is stopped: $said"
step "doctor says it is stopped, and nothing it stopped fails it"

for command in \
	"upgrade-deployment --yes" \
	"rollback-deployment --yes" \
	"restore --to-time 2026-01-01T00:00:00Z --yes" \
	"up --id DEP-01ARZ3NDEKTSV4RRFFQ69G5FAV --no-doctor"; do
	# shellcheck disable=SC2086
	run $command
	[ "$code" = 1 ] || failed "\`meridian $command\` exited $code while stopped: $said"
	printf '%s' "$said" | grep -qF '`meridian start` starts it as it was' \
		|| failed "\`meridian $command\` does not point to meridian start: $said"
done
[ "$(replicas "statefulset/$DB")" = 0 ] || failed "a refused command started the database"
step "up, upgrade-deployment, rollback-deployment and restore refused, pointing to start"

run stop --yes
[ "$code" = 0 ] || failed "stop again exited $code: $said"
printf '%s' "$said" | grep -qF "is stopped already" || failed "stop again did not say so: $said"
step "stop again: stopped already, nothing changed"

run start
[ "$code" = 0 ] || failed "start exited $code: $said"
[ "$(counts)" = "$before" ] || failed "start did not give back every count and schedule:
$(counts)
not
$before"
[ -z "$(record)" ] || failed "start left its record"
step "started: every count and every schedule as it was, the record removed"

run start
[ "$code" = 0 ] && printf '%s' "$said" | grep -qF "is not stopped" \
	|| failed "start on a running deployment: $code $said"

# ── Cut short, and run again ─────────────────────────────────────────────────

# The dashboard takes its grace to stop: a stop given 5s does not finish.
run stop --yes --timeout 5s
[ "$code" = 1 ] || failed "a stop given 5s finished: $said"
printf '%s' "$said" | grep -qF "again finishes the stop" || failed "the cut stop does not say what finishes it: $said"
[ -n "$(record)" ] || failed "the cut stop left no record"
[ "$(replicas "statefulset/$DB")" = 1 ] || failed "the cut stop reached the database"
run stop --yes
[ "$code" = 0 ] || failed "stop again after a cut one exited $code: $said"
[ "$(replicas "statefulset/$DB")" = 0 ] || failed "stop again did not stop the database"
printf '%s' "$(record)" | grep -qF '["meridian-meridian-runtime-dashboard",2]' \
	|| failed "the record lost the dashboard's count: $(record)"
step "a stop cut short, finished by stop again, its counts as first recorded"

# The dashboard takes 15s to be ready: a start given 5s does not finish.
run start --timeout 5s
[ "$code" = 1 ] || failed "a start given 5s finished: $said"
[ -n "$(record)" ] || failed "the cut start removed its record"
run start
[ "$code" = 0 ] || failed "start again after a cut one exited $code: $said"
[ "$(counts)" = "$before" ] || failed "start again did not give back every count and schedule"
[ -z "$(record)" ] || failed "start again left its record"
step "a start cut short, finished by start again"

echo "stop-start OK: the round trip, each refusal, a stop and a start cut short and run again, status, doctor and the refusals while stopped, on k3d"
