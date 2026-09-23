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


def _check_host_nvidia_present() -> bool:
    """Check if NVIDIA GPU drivers or utilities exist on the host OS."""
    import shutil
    if shutil.which("nvidia-smi") is not None:
        return True
    if sys.platform == "win32":
        sys_root = os.environ.get("SystemRoot", r"C:\Windows")
        dll_path = os.path.join(sys_root, "System32", "nvcuda.dll")
        if os.path.exists(dll_path):
            return True
    elif sys.platform.startswith("linux"):
        if os.path.exists("/proc/driver/nvidia/version"):
            return True
        for lib in ("/usr/lib/x86_64-linux-gnu/libcuda.so", "/usr/lib64/libcuda.so", "/usr/lib/libcuda.so"):
            if os.path.exists(lib):
                return True
    return False


def detect_hardware() -> Dict[str, Any]:
    """Detect local compute hardware and return selection details, detection chain, and signature."""
    system = sys.platform
    machine = platform.machine().lower()
    details: Dict[str, Any] = {
        "platform": system,
        "machine": machine,
        "python_version": platform.python_version(),
    }
    detection_chain: list[Dict[str, Any]] = []

    tier = "cpu"
    device = "cpu"
    device_name = "CPU"
    reason = "Default CPU execution"

    # 1. Probe NVIDIA GPU / CUDA
    cuda_detected = False
    cuda_host_present = _check_host_nvidia_present()

    try:
        import torch
        details["torch_version"] = torch.__version__
        cuda_version = getattr(torch.version, "cuda", None)
        details["torch_cuda_version"] = cuda_version

        if torch.cuda.is_available():
            try:
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
                    "cuda_version": cuda_version,
                }
                cuda_detected = True
                detection_chain.append({
                    "probe": "nvidia_cuda",
                    "status": "pass",
                    "message": f"NVIDIA GPU confirmed: {gpu_name} ({device_count} device(s), CUDA {cuda_version})",
                })
            except Exception as e:
                err_msg = f"torch.cuda.is_available() was True, but get_device_name(0) failed to resolve hardware: {e}"
                detection_chain.append({
                    "probe": "nvidia_cuda",
                    "status": "fail",
                    "message": err_msg,
                })
        else:
            if cuda_host_present:
                err_msg = (
                    f"NVIDIA hardware/driver detected on host, but installed PyTorch ({torch.__version__}) "
                    f"is a CPU-only build (torch.version.cuda is None). To enable GPU acceleration, install a "
                    f"CUDA-enabled PyTorch wheel: pip install torch --index-url https://download.pytorch.org/whl/cu121"
                )
                detection_chain.append({
                    "probe": "nvidia_cuda",
                    "status": "misconfigured",
                    "message": err_msg,
                })
                details["cuda_misconfiguration"] = err_msg
            else:
                detection_chain.append({
                    "probe": "nvidia_cuda",
                    "status": "not_present",
                    "message": "No NVIDIA GPU hardware or driver detected on host; torch.cuda.is_available() is False",
                })
    except ImportError as e:
        detection_chain.append({
            "probe": "nvidia_cuda",
            "status": "fail",
            "message": f"PyTorch import failed: {e}",
        })

    # 2. Probe Apple Silicon (macOS on arm64/aarch64) if CUDA not active
    if not cuda_detected and system == "darwin" and machine in ("arm64", "aarch64"):
        mac_ver_str = platform.mac_ver()[0]
        major_mac_ver = 0
        try:
            major_mac_ver = int(mac_ver_str.split(".")[0])
        except (ValueError, IndexError):
            pass

        if major_mac_ver < 14:
            detection_chain.append({
                "probe": "apple_silicon_os_version",
                "status": "fail",
                "message": f"macOS {mac_ver_str} detected. macOS 14+ (Sonoma) is required for unified memory acceleration; falling back to CPU.",
            })
            tier = "cpu"
            device = "cpu"
            device_name = f"Apple Silicon CPU (macOS {mac_ver_str})"
            reason = f"Apple Silicon CPU fallback: macOS {mac_ver_str} is below required macOS 14+"
        else:
            detection_chain.append({
                "probe": "apple_silicon_os_version",
                "status": "pass",
                "message": f"macOS {mac_ver_str} verified (>= 14 Sonoma)",
            })

            # Check laya-mlx specifically for laya-typed-decisions
            # laya-mlx (mizorewww/laya-mlx) only provides weights for base model (aac6fef/laya-mlx),
            # NOT for convaiinnovations/laya-typed-decisions.
            mlx_available_for_checkpoint = False
            try:
                import laya_mlx  # type: ignore
                # Probe if laya-typed-decisions converted weights exist
                detection_chain.append({
                    "probe": "apple_silicon_mlx",
                    "status": "checkpoint_unsupported",
                    "message": (
                        "laya-mlx is installed, but laya-mlx weights are only published for base laya (aac6fef/laya-mlx), "
                        "not for convaiinnovations/laya-typed-decisions. Falling back to standard laya PyTorch runtime "
                        "rather than silently substituting checkpoint."
                    ),
                })
            except ImportError:
                detection_chain.append({
                    "probe": "apple_silicon_mlx",
                    "status": "not_installed",
                    "message": "laya-mlx package is not installed",
                })

            # Check PyTorch MPS backend
            mps_available = False
            mps_reason = "MPS not available"
            try:
                import torch
                if hasattr(torch.backends, "mps") and torch.backends.mps.is_available():
                    try:
                        test_tensor = torch.zeros(1, device="mps")
                        del test_tensor
                        mps_available = True
                        tier = "apple_silicon_mps"
                        device = "mps"
                        device_name = f"Apple Silicon (MPS: {platform.processor() or 'ARM64'})"
                        reason = "Detected Apple Silicon with PyTorch MPS backend verified"
                        details["apple_silicon"] = {"backend": "mps", "verified": True}
                        detection_chain.append({
                            "probe": "apple_silicon_mps",
                            "status": "pass",
                            "message": "PyTorch MPS backend available and tensor allocation verified",
                        })
                    except Exception as e:
                        mps_reason = f"torch.backends.mps.is_available() was True, but tensor allocation failed: {e}"
                        detection_chain.append({
                            "probe": "apple_silicon_mps",
                            "status": "fail",
                            "message": mps_reason,
                        })
                else:
                    mps_reason = "torch.backends.mps.is_available() is False"
                    detection_chain.append({
                        "probe": "apple_silicon_mps",
                        "status": "fail",
                        "message": mps_reason,
                    })
            except Exception as e:
                mps_reason = f"MPS probe exception: {e}"
                detection_chain.append({
                    "probe": "apple_silicon_mps",
                    "status": "fail",
                    "message": mps_reason,
                })

            if not mps_available:
                tier = "cpu"
                device = "cpu"
                device_name = f"Apple Silicon CPU ({platform.processor() or 'ARM64'})"
                reason = f"Apple Silicon detected, but neither laya-mlx nor verified MPS is active; falling back to CPU ({mps_reason})"
                details["apple_silicon"] = {"backend": "cpu_fallback", "reason": mps_reason}

    # 3. AMD / ROCm check: treat as CPU-tier unless explicitly verified
    if not cuda_detected and tier == "cpu":
        try:
            import torch
            is_hip = getattr(torch.version, "hip", None) is not None
            if is_hip and torch.cuda.is_available():
                detection_chain.append({
                    "probe": "amd_rocm",
                    "status": "unverified",
                    "message": "ROCm/HIP PyTorch build detected, but treated as CPU-tier pending validated hardware qualification",
                })
            else:
                detection_chain.append({
                    "probe": "amd_rocm",
                    "status": "not_present",
                    "message": "No AMD ROCm/HIP device detected",
                })
        except Exception:
            pass

    # 4. CPU details and capabilities
    if device == "cpu":
        detection_chain.append({
            "probe": "cpu_runtime",
            "status": "active",
            "message": f"Operating on CPU ({os.cpu_count() or 1} cores)",
        })
        try:
            import torch
            cpu_info = {
                "logical_cores": os.cpu_count() or 1,
                "mkl_available": getattr(torch.backends.mkl, "is_available", lambda: False)(),
                "mkldnn_available": getattr(torch.backends.mkldnn, "is_available", lambda: False)(),
                "openmp_available": getattr(torch.backends.openmp, "is_available", lambda: False)(),
            }
            details["cpu"] = cpu_info
            if not (system == "darwin" and machine in ("arm64", "aarch64")):
                device_name = f"CPU ({platform.processor() or platform.machine()}, {cpu_info['logical_cores']} cores)"
                reason = f"CPU execution active: {cpu_info['logical_cores']} cores (MKL={cpu_info['mkl_available']}, oneDNN={cpu_info['mkldnn_available']})"
        except Exception:
            pass

    # 5. Generate deterministic hardware signature
    # Hash components that affect inference performance/runtime
    laya_version = "unknown"
    try:
        import laya
        laya_version = getattr(laya, "__version__", "1.0.0")
    except Exception:
        pass
    details["laya_version"] = laya_version
    details["detection_chain"] = detection_chain

    sig_raw = f"{system}:{machine}:{tier}:{device_name}:{details.get('torch_version', 'none')}:{laya_version}"
    signature = hashlib.sha256(sig_raw.encode("utf-8")).hexdigest()[:16]

    result = {
        "tier": tier,
        "device": device,
        "device_name": device_name,
        "reason": reason,
        "signature": signature,
        "details": details,
        "detection_chain": detection_chain,
    }

    logger.info(f"Hardware detection selected tier '{tier}' on device '{device_name}' [signature: {signature}]: {reason}")
    for probe in detection_chain:
        logger.info(f"  Probe [{probe['probe']}]: status={probe['status']} - {probe['message']}")
    return result
