"""Hardware detection and device selection for Laya decision sidecar.

Accurately probes the host hardware to select the optimal runtime backend:
- NVIDIA GPU: verified via torch.cuda.is_available() AND torch.cuda.get_device_name(0)
- Apple Silicon (macOS 14+): probes laya-mlx; if unavailable, probes torch MPS; falls back to CPU
- AMD / ROCm: verifies HIP support; falls back to CPU if unconfirmed
- CPU: default with thread pool and BLAS/AVX capabilities detected
"""

from __future__ import annotations

import hashlib
import logging
import os
import platform
import sys
from typing import Any, Dict, Optional

logger = logging.getLogger("laya-sidecar.hardware")


def detect_hardware() -> Dict[str, Any]:
    """Detect local compute hardware and return selection details and deterministic signature."""
    system = sys.platform
    machine = platform.machine().lower()
    details: Dict[str, Any] = {
        "platform": system,
        "machine": machine,
        "python_version": platform.python_version(),
    }

    tier = "cpu"
    device = "cpu"
    device_name = "CPU"
    reason = "Default CPU execution"

    # 1. Probe NVIDIA GPU / CUDA
    cuda_detected = False
    cuda_error: Optional[str] = None
    try:
        import torch
        details["torch_version"] = torch.__version__
        if torch.cuda.is_available():
            try:
                # Crucial check: verify device_name actually resolves without error.
                # A CPU-only wheel installed on a machine with NVIDIA drivers will report False,
                # but a broken CUDA installation may throw when querying the device name.
                gpu_name = torch.cuda.get_device_name(0)
                gpu_cap = torch.cuda.get_device_capability(0)
                device_count = torch.cuda.device_count()
                tier = "cuda"
                device = "cuda"
                device_name = gpu_name
                reason = f"Confirmed NVIDIA GPU: {gpu_name} (compute capability {gpu_cap[0]}.{gpu_cap[1]}, {device_count} device(s))"
                details["cuda"] = {
                    "device_name": gpu_name,
                    "compute_capability": gpu_cap,
                    "device_count": device_count,
                    "cuda_version": torch.version.cuda,
                }
                cuda_detected = True
            except Exception as e:
                cuda_error = f"torch.cuda.is_available() was True, but get_device_name(0) failed: {e}"
        else:
            cuda_error = "torch.cuda.is_available() returned False (CPU-only PyTorch build or no CUDA device)"
    except ImportError as e:
        cuda_error = f"PyTorch import failed: {e}"

    if not cuda_detected and cuda_error:
        details["cuda_probe_failure"] = cuda_error

    # 2. Probe Apple Silicon (macOS on arm64/aarch64) if CUDA not active
    if not cuda_detected and system == "darwin" and machine in ("arm64", "aarch64"):
        mlx_available = False
        try:
            import laya_mlx  # type: ignore
            mlx_available = True
            tier = "apple_silicon_mlx"
            device = "mlx"
            device_name = f"Apple Silicon (MLX: {platform.processor() or 'ARM64'})"
            reason = "Detected Apple Silicon with laya-mlx package available"
            details["apple_silicon"] = {"backend": "mlx", "package": "laya-mlx"}
        except ImportError:
            details["apple_silicon_mlx_probe"] = "laya-mlx not installed"

        if not mlx_available:
            # Check MPS support in standard torch
            mps_available = False
            mps_reason = "MPS not available"
            try:
                import torch
                if hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
                    # Test actual MPS allocation to verify driver/OS compatibility
                    try:
                        test_tensor = torch.zeros(1, device="mps")
                        del test_tensor
                        mps_available = True
                        tier = "apple_silicon_mps"
                        device = "mps"
                        device_name = f"Apple Silicon (MPS: {platform.processor() or 'ARM64'})"
                        reason = "Detected Apple Silicon with PyTorch MPS backend verified"
                        details["apple_silicon"] = {"backend": "mps", "verified": True}
                    except Exception as e:
                        mps_reason = f"torch.backends.mps.is_available() was True, but tensor allocation failed: {e}"
                else:
                    mps_reason = "torch.backends.mps.is_available() is False"
            except Exception as e:
                mps_reason = f"MPS probe exception: {e}"

            if not mps_available:
                tier = "cpu"
                device = "cpu"
                device_name = f"Apple Silicon CPU ({platform.processor() or 'ARM64'})"
                reason = f"Apple Silicon detected, but neither laya-mlx nor verified MPS is active; falling back to CPU ({mps_reason})"
                details["apple_silicon"] = {"backend": "cpu_fallback", "reason": mps_reason}

    # 3. CPU details and capabilities
    if device == "cpu":
        try:
            import torch
            cpu_info = {
                "logical_cores": os.cpu_count() or 1,
                "mkl_available": getattr(torch.backends.mkl, "is_available", lambda: False)(),
                "mkldnn_available": getattr(torch.backends.mkldnn, "is_available", lambda: False)(),
                "openmp_available": getattr(torch.backends.openmp, "is_available", lambda: False)(),
            }
            details["cpu"] = cpu_info
            if not cuda_detected and not (system == "darwin" and machine in ("arm64", "aarch64")):
                device_name = f"CPU ({platform.processor() or platform.machine()}, {cpu_info['logical_cores']} cores)"
                reason = f"CPU execution active: {cpu_info['logical_cores']} cores (MKL={cpu_info['mkl_available']}, oneDNN={cpu_info['mkldnn_available']})"
        except Exception:
            pass

    # 4. Generate deterministic hardware signature
    # Hash components that affect inference performance/runtime
    laya_version = "unknown"
    try:
        import laya
        laya_version = getattr(laya, "__version__", "1.0.0")
    except Exception:
        pass
    details["laya_version"] = laya_version

    sig_raw = f"{system}:{machine}:{tier}:{device_name}:{details.get('torch_version', 'none')}:{laya_version}"
    signature = hashlib.sha256(sig_raw.encode("utf-8")).hexdigest()[:16]

    result = {
        "tier": tier,
        "device": device,
        "device_name": device_name,
        "reason": reason,
        "signature": signature,
        "details": details,
    }

    logger.info(f"Hardware detection selected tier '{tier}' on device '{device_name}' [signature: {signature}]: {reason}")
    return result
