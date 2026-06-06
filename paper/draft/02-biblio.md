# Bibliography — seed

**Status:** seed list. Trim + format to venue BibTeX style after section bodies stabilize.

## Residual / predictive coding (video, classical)

- Wiegand, T., Sullivan, G.J., Bjøntegaard, G., Luthra, A. **Overview of the H.264/AVC video coding standard.** IEEE TCSVT 2003.
- Sullivan, G.J., Ohm, J.-R., Han, W.-J., Wiegand, T. **Overview of the High Efficiency Video Coding (HEVC) Standard.** IEEE TCSVT 2012.
- Chen, Y. et al. **An Overview of Core Coding Tools in the AV1 Video Codec.** PCS 2018.

## Neural image compression

- Toderici, G. et al. **Variable Rate Image Compression with Recurrent Neural Networks.** ICLR 2016.
- Ballé, J., Laparra, V., Simoncelli, E.P. **End-to-end Optimized Image Compression.** ICLR 2017.
- Ballé, J., Minnen, D., Singh, S., Hwang, S.J., Johnston, N. **Variational image compression with a scale hyperprior.** ICLR 2018.
- Mentzer, F. et al. **High-Fidelity Generative Image Compression.** NeurIPS 2020. (the "HiFiC" paper)
- Cheng, Z., Sun, H., Takeuchi, M., Katto, J. **Learned Image Compression with Discretized Gaussian Mixture Likelihoods and Attention Modules.** CVPR 2020.

## Lossless + scalable codecs

- Alakuijala, J. et al. **JPEG XL next-generation image compression architecture and coding tools.** SPIE 2019.
- Sneyers, J., Wuille, P. **FLIF: Free Lossless Image Format Based on MANIAC Compression.** ICIP 2016.
- Taubman, D.S., Marcellin, M.W. **JPEG2000: Image Compression Fundamentals, Standards and Practice.** Springer 2002. (the scalable / lossless-on-top-of-lossy lineage)

## Super-resolution (predictor candidates)

- Wang, X. et al. **Real-ESRGAN: Training Real-World Blind Super-Resolution with Pure Synthetic Data.** ICCV Workshops 2021.
- Lim, B., Son, S., Kim, H., Nah, S., Lee, K.M. **Enhanced Deep Residual Networks for Single Image Super-Resolution.** CVPRW 2017. (EDSR)
- Ledig, C. et al. **Photo-Realistic Single Image Super-Resolution Using a Generative Adversarial Network.** CVPR 2017. (SRGAN)

## Game asset / texture compression

- Real-Time Rendering 4e (Akenine-Möller et al.) — texture compression chapter, BC overview
- van Waveren, J.M.P., Castaño, I. **Real-time DXT compression.** Tech report Id Software 2006.
- Iourcha, K., Nayak, K.S., Hong, Z. **System and method for fixed-rate block-based image compression with inferred pixel values.** US Patent 5,956,431, 1999. (BC1)
- Castaño, I. **High quality DXT compression using OpenCL.** id Software 2008. (etc.)

## Delta / patch encoding

- MacDonald, J. **File system support for delta compression.** USENIX 2000. (xdelta lineage)
- Tridgell, A., Mackerras, P. **The rsync algorithm.** ANU TR-CS-96-05, 1996.
- Percival, C. **Naïve differences of executable code.** 2003. (bsdiff)

## Anti-cheat / game-asset integrity (sparse academic; cite white papers)

- BattlEye, Easy Anti-Cheat — vendor whitepapers re file-hash integrity checks (cite product docs, not academic)
- Discussion on FitGirl-style repacks and reversibility (community knowledge; weave into prior-art prose, not formal cites)

## Quantization

- Gray, R.M., Neuhoff, D.L. **Quantization.** IEEE Trans. Inf. Theory 1998. (classical)
- Jacob, B. et al. **Quantization and Training of Neural Networks for Efficient Integer-Arithmetic-Only Inference.** CVPR 2018. (not directly used; cite if quant_step >1 evaluation pulls on it)

## To investigate (not yet read)

- JPEG-AI working group drafts (2024 onward) — neural codecs becoming standardized; check for overlap
- AVIF and HEIC lossless modes — comparison baselines?
- Recent NeurIPS 2024/2025 papers on neural residual coding (search needed)
