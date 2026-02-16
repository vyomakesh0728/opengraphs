"""Demo training script for OpenGraphs."""

import math
import os
import random
import subprocess
import sys
import time

try:
    import psutil
except Exception:
    psutil = None

# ── Hyperparameters ──────────────────────────────────────────────────────
# Intentionally bad defaults for the demo run:
# 1) base LR is too high
# 2) warmup is too short
# 3) LR jumps too high after warmup
# 4) batch is too small (extra noisy updates)
LEARNING_RATE = float(os.getenv("DEMO_LR", "0.001"))
TOTAL_STEPS = 100
BATCH_SIZE = int(os.getenv("DEMO_BATCH", "32"))
SLEEP_PER_STEP = 0.3
WARMUP_STEPS = 20
PEAK_LR_MULT = 1.0

# ── Helpers ──────────────────────────────────────────────────────────────

def get_tb_writer():
    log_dir = os.getenv("TB_LOG_DIR", "runs/demo")
    if os.getenv("ENABLE_TB", "0") != "1":
        return None
    try:
        from tensorboardX import SummaryWriter
        return SummaryWriter(log_dir=log_dir)
    except ImportError:
        pass
    try:
        from torch.utils.tensorboard import SummaryWriter
        return SummaryWriter(log_dir=log_dir)
    except ImportError:
        pass
    return None


def report(metric, value, step):
    try:
        from og_agent_chat.client import send_metric
        send_metric(metric, value, step=step)
    except Exception:
        pass


def log(writer, tag, value, step):
    if writer is not None:
        writer.add_scalar(tag, value, step)
        writer.flush()


def gpu_stats():
    try:
        out = subprocess.run(
            ["nvidia-smi", "--query-gpu=temperature.gpu,utilization.gpu,memory.used,memory.total,power.draw",
             "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=2,
        )
        parts = [p.strip() for p in out.stdout.strip().split(",")]
        if len(parts) >= 5:
            return {
                "gpu/temp_c": float(parts[0]),
                "gpu/util_pct": float(parts[1]),
                "gpu/mem_used_mb": float(parts[2]),
                "gpu/mem_total_mb": float(parts[3]),
                "gpu/power_w": float(parts[4]),
            }
    except Exception:
        pass
    return {}


def cpu_stats():
    if psutil is None:
        return {}
    mem = psutil.virtual_memory()
    return {
        "sys/cpu_pct": psutil.cpu_percent(interval=None),
        "sys/ram_used_mb": mem.used / (1024 * 1024),
        "sys/ram_total_mb": mem.total / (1024 * 1024),
    }


# ── Training loop ────────────────────────────────────────────────────────
#
# This demo intentionally starts unstable so the assistant has something to fix.
# The most important fixes are:
# - LEARNING_RATE: 0.008 -> 0.001
# - WARMUP_STEPS: 3 -> 20
# - PEAK_LR_MULT: 4.0 -> 1.0
# Optional quality-of-life fix for cleaner curves:
# - BATCH_SIZE: 8 -> 32

def scheduled_lr(step, base_lr):
    if step <= WARMUP_STEPS:
        return base_lr * (0.2 + 0.8 * step / max(1, WARMUP_STEPS))
    # Keep LR too high after warmup on purpose.
    return base_lr * PEAK_LR_MULT

def train():
    lr = LEARNING_RATE
    writer = get_tb_writer()
    small_batch = BATCH_SIZE < 16
    noise_std = 0.05 if small_batch else 0.02

    print(f"[info] starting training: lr={lr}, steps={TOTAL_STEPS}, batch={BATCH_SIZE}")
    sys.stdout.flush()

    loss = 2.5
    acc = 0.10
    if psutil is not None:
        psutil.cpu_percent(interval=None)

    for step in range(1, TOTAL_STEPS + 1):
        noise = random.gauss(0, noise_std)
        eff_lr = scheduled_lr(step, lr)

        if eff_lr >= 0.005:
            # LR is too high: short early improvement, then instability.
            if step <= 25:
                decay = math.exp(-eff_lr * step * 0.8)
                loss = 2.5 * decay + noise
                acc = min(0.60, 0.10 + (1 - decay) * 0.50 + noise * 0.35)
            else:
                loss = loss + (0.03 if small_batch else 0.02) + random.gauss(0, 0.03)
                acc = acc - (0.003 if small_batch else 0.002) + random.gauss(0, 0.004)
        else:
            # Stable LR: converges more smoothly.
            decay = math.exp(-eff_lr * step * 3.0)
            loss = 2.5 * decay + noise * 0.45
            acc = min(0.95, 0.10 + (1 - decay) * 0.85 + noise * 0.25)
            if BATCH_SIZE < 16:
                # Keep an obvious "not fully stable" signal when batch is tiny.
                acc = min(acc, 0.88)

        loss = max(0.01, loss)
        acc = max(0.0, min(1.0, acc))
        grad_norm = abs(random.gauss(0.5, 0.25 if small_batch else 0.2))
        if eff_lr >= 0.005 and step > 25:
            grad_norm += random.uniform(0.5, 0.9) if small_batch else random.uniform(0.3, 0.8)
        throughput = BATCH_SIZE / (SLEEP_PER_STEP + random.uniform(0, 0.05))

        report("train/loss", loss, step)
        report("train/accuracy", acc, step)
        report("train/lr_effective", eff_lr, step)
        report("train/grad_norm", grad_norm, step)
        report("train/throughput", throughput, step)
        log(writer, "train/loss", loss, step)
        log(writer, "train/accuracy", acc, step)
        log(writer, "train/lr_effective", eff_lr, step)
        log(writer, "train/grad_norm", grad_norm, step)
        log(writer, "train/throughput", throughput, step)

        if step % 5 == 0:
            for tag, val in gpu_stats().items():
                report(tag, val, step)
                log(writer, tag, val, step)
            for tag, val in cpu_stats().items():
                report(tag, val, step)
                log(writer, tag, val, step)

        print(
            f"step {step:>4d} | loss={loss:.4f} | acc={acc:.4f} | "
            f"base_lr={lr:.4f} | eff_lr={eff_lr:.4f}"
        )
        sys.stdout.flush()
        time.sleep(SLEEP_PER_STEP)

    print("[info] training complete")
    if writer is not None:
        writer.flush()
        writer.close()


if __name__ == "__main__":
    train()
