# reference-plugin

A Meridian plugin, made by `meridian plugin new`.

It runs beside a sidecar in a Meridian deployment and reaches nothing else.
Who it is and what it may publish and subscribe to come from how the
deployment launches it, never from here.

    docker build -t reference-plugin .

Then launch it in a deployment as a `sidecars[]` entry naming this image and
the roles its grants come from. It holds none as it stands, and is admitted
with no topics until it names some.
