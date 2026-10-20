#!/usr/bin/env python3
"""Write the harness's shipped scenarios.

The scenarios are values: they are written once, committed, and read by
the gate. This script is the *authoring* tool that produced them; it is
kept so that a change to a scenario's shape is a change to one place.

Usage: python3 scripts/write-scenarios.py <out-dir>
"""

import json
import os
import sys


def direction(rate_bps, propagation_us, jitter, loss, reorder=0, duplicate=0,
              queue_cap=None, burst=None):
    return {
        "rate_bps": rate_bps,
        "propagation_us": propagation_us,
        "jitter": jitter,
        "loss": loss,
        "reorder_permille": reorder,
        "duplicate_permille": duplicate,
        "queue_cap_bytes": queue_cap if queue_cap is not None else rate_bps // 8 * 2,
        "burst_bytes": burst if burst is not None else rate_bps // 8,
        "reorder_window_us": 10_000,
        "duplicate_delay_us": 1_000,
        "extra_delay_us": 0,
    }


TIERS = {
    "T0": {"down": (100_000_000, 5_000, {"Normal": {"amplitude_us": 2_000}}, {"Uniform": {"permille": 0}}),
           "up": (10_000_000, 5_000, {"Normal": {"amplitude_us": 2_000}}, {"Uniform": {"permille": 0}}),
           "mtu": 65_536},
    "T1": {"down": (5_000_000, 25_000, {"Normal": {"amplitude_us": 10_000}}, {"Uniform": {"permille": 1}}),
           "up": (1_000_000, 25_000, {"Normal": {"amplitude_us": 10_000}}, {"Uniform": {"permille": 1}}),
           "mtu": 65_536},
    "T2": {"down": (1_000_000, 75_000, {"Normal": {"amplitude_us": 40_000}}, {"Uniform": {"permille": 20}}),
           "up": (256_000, 75_000, {"Normal": {"amplitude_us": 40_000}}, {"Uniform": {"permille": 20}}),
           "mtu": 65_536},
    "T3": {"down": (256_000, 200_000, {"Normal": {"amplitude_us": 100_000}}, {"Uniform": {"permille": 100}}),
           "up": (128_000, 200_000, {"Normal": {"amplitude_us": 100_000}}, {"Uniform": {"permille": 100}}),
           "mtu": 65_536},
    "T4": {"down": (32_000, 700_000, {"Pareto": {"amplitude_us": 500_000}}, {"Uniform": {"permille": 350}}),
           "up": (32_000, 700_000, {"Pareto": {"amplitude_us": 500_000}}, {"Uniform": {"permille": 350}}),
           "mtu": 1_150},
}


def profile(tier, seed=1, outages=None, down=None, up=None, mtu=None):
    spec = TIERS[tier]
    down_spec = spec["down"]
    up_spec = spec["up"]
    return {
        "down": direction(*down_spec, **({"queue_cap": down[2], "burst": down[3]} if down else {})),
        "up": direction(*up_spec, **({"queue_cap": up[2], "burst": up[3]} if up else {})),
        "mtu_bytes": mtu if mtu is not None else spec["mtu"],
        "outages": {"windows": outages or []},
        "seed": seed,
    }


def scenario(name, tier, steps, link=None, seed=1, faults=None):
    document = {
        "schema": 1,
        "name": name,
        "tier": tier,
        "seed": seed,
        "link": link if link is not None else profile(tier, seed),
        "grid": {"width_px": 1920, "height_px": 1080, "cell_size_px": 64},
        "steps": steps,
    }
    if faults:
        document["faults"] = faults
    return document


def advance(ms):
    return {"Advance": {"ms": ms}}


def change(x, y, w, h):
    return {"CaptureChange": {"region": {"x_px": x, "y_px": y, "width_px": w, "height_px": h}}}


def key(code, pressed):
    return {"ViewerInput": {"event": {"Key": {"code": code, "pressed": pressed}}}}


def assert_(invariant):
    return {"Assert": {"invariant": invariant}}


def typing(codes, gap_ms):
    steps = []
    for code in codes:
        steps.append(key(code, True))
        steps.append(key(code, False))
        steps.append(advance(gap_ms))
    return steps


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "scenarios"
    os.makedirs(out, exist_ok=True)
    written = []

    def write(document):
        path = os.path.join(out, document["name"] + ".json")
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(document, handle, indent=2, sort_keys=False)
            handle.write("\n")
        written.append(path)

    # ── the baselines: one per tier ───────────────────────────────────────
    for tier in TIERS:
        rtt = {"T0": 10, "T1": 50, "T2": 150, "T3": 400, "T4": 1400}[tier]
        steps = [advance(2 * rtt), change(320, 240, 640, 200), advance(rtt)]
        steps += typing(["KeyH", "KeyE", "KeyL", "KeyO"], max(50, rtt // 2))
        steps += [advance(4 * rtt + 2000), assert_("INV-2")]
        write(scenario("baseline-" + tier.lower(), tier, steps))

    # ── the typing echo: the tier's echo promise ─────────────────────────
    for tier in ("T1", "T2", "T3"):
        rtt = {"T1": 50, "T2": 150, "T3": 400}[tier]
        steps = [advance(2 * rtt), change(320, 240, 640, 200), advance(rtt)]
        steps += typing(["KeyT", "KeyE", "KeyX", "KeyT"], max(50, rtt // 2))
        steps += [advance(4 * rtt + 2000), assert_("INV-8"), assert_("INV-2")]
        write(scenario("typing-echo-" + tier.lower(), tier, steps))

    # ── the outage: a blackhole, and the session's survival ──────────────
    outage = [{"start_us": 2_000_000, "duration_us": 3_000_000, "kind": "Blackhole"}]
    steps = [advance(1_000), change(320, 240, 640, 200), advance(1_000)]
    steps += typing(["KeyA"], 200)
    steps += [advance(8_000), assert_("INV-9"), assert_("INV-2")]
    write(scenario("outage-blackhole-t3", "T3", steps,
                   link=profile("T3", seed=3, outages=outage)))

    # ── the one-way blackhole: the uplink dies, the downlink lives ───────
    outage = [{"start_us": 1_500_000, "duration_us": 2_000_000,
               "kind": {"OneWayBlackhole": {"direction": "Up"}}}]
    steps = [advance(1_000), change(320, 240, 640, 200), advance(1_000)]
    steps += typing(["KeyA", "KeyB"], 200)
    steps += [advance(8_000), assert_("INV-4")]
    write(scenario("outage-one-way-t2", "T2", steps,
                   link=profile("T2", seed=4, outages=outage)))

    # ── the flap: five cycles of ten seconds down and ten up ─────────────
    outage = [{"start_us": 2_000_000 + index * 20_000_000, "duration_us": 10_000_000,
               "kind": "Blackhole"} for index in range(5)]
    steps = [advance(1_000), change(320, 240, 640, 200), advance(1_000)]
    steps += [advance(120_000), assert_("INV-9"), assert_("INV-4")]
    write(scenario("flap-five-cycles-t3", "T3", steps,
                   link=profile("T3", seed=5, outages=outage)))

    # ── the handover: the carrier changes under the link's feet ──────────
    handover_link = profile("T2", seed=6)
    next_link = profile("T1", seed=6)
    steps = [advance(1_000), change(320, 240, 640, 200), advance(2_000)]
    steps.append({"Handover": {"handover": {
        "profile": next_link, "split_permille": 500, "loss_permille": 200}}})
    steps += typing(["KeyA"], 200)
    steps += [advance(6_000), assert_("INV-2"), assert_("INV-10")]
    write(scenario("handover-t2-to-t1", "T2", steps, link=handover_link))

    # ── the reordering: the link displaces objects out of order ──────────
    reordered = profile("T2", seed=7)
    reordered["down"]["reorder_permille"] = 200
    reordered["up"]["reorder_permille"] = 100
    steps = [advance(1_000), change(320, 240, 640, 200), advance(1_000)]
    steps += typing(["KeyA", "KeyB", "KeyC"], 200)
    steps += [advance(6_000), assert_("INV-1"), assert_("INV-2")]
    write(scenario("reordering-t2", "T2", steps, link=reordered))

    # ── the lossy link: the tier's loss, and the attribution ─────────────
    lossy = profile("T3", seed=8)
    lossy["down"]["loss"] = {"Bursty": {"permille": 100}}
    lossy["up"]["loss"] = {"Bursty": {"permille": 100}}
    steps = [advance(1_000), change(320, 240, 640, 200), advance(1_000)]
    steps += typing(["KeyA", "KeyB"], 200)
    steps += [advance(8_000), assert_("INV-10"), assert_("INV-2")]
    write(scenario("lossy-bursty-t3", "T3", steps, link=lossy))

    # ── the broken peers: one scenario per deliberate defect ─────────────
    for name, invariant, faults, keys in (
        ("broken-partial-update", "INV-1", {"apply_partial_update": True}, 2),
        ("broken-unemitted-cell", "INV-2", {"acknowledge_unemitted_cell": True}, 2),
        ("broken-input-twice", "INV-3", {"apply_input_twice": True}, 2),
        ("broken-stuck-key", "INV-4", {"keep_key_held_after_release": True}, 2),
        ("broken-unbounded-queue", "INV-5", {"unbounded_queue": True}, 70),
        ("broken-oversize-object", "INV-6", {"oversize_object": True}, 2),
        ("broken-burst-over-budget", "INV-7", {"burst_over_budget": True}, 2),
        ("broken-slow-echo", "INV-8", {"delay_echo": True}, 2),
        ("broken-recreated-session", "INV-9", {"recreate_session": True}, 2),
        ("broken-unattributed-drop", "INV-10", {"unattributed_drop": True}, 2),
    ):
        steps = [advance(1_000), change(320, 240, 640, 200), advance(1_000)]
        steps += typing(["KeyA"] * keys, 20)
        steps += [{"ViewerDisconnect": None}, advance(4_000), {"ViewerReconnect": None}]
        steps += [advance(6_000), assert_(invariant)]
        write(scenario(name, "T2", steps, faults=faults))

    for path in written:
        print("wrote", path)


if __name__ == "__main__":
    main()
