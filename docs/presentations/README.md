# Presentations

演示稿源文件放在这个目录。

当前演示：

- `kernel-dev-criew-onboarding.json`
- `kernel-dev-criew-onboarding.pptx`
- `kernel-dev-criew-onboarding-beamer.tex`
- `kernel-dev-criew-onboarding-beamer.pdf`

重新生成：

```bash
python3 scripts/generate-pptx.py \
  docs/presentations/kernel-dev-criew-onboarding.json \
  docs/presentations/kernel-dev-criew-onboarding.pptx
```

生成 Beamer PDF：

```bash
xelatex \
  -interaction=nonstopmode \
  -halt-on-error \
  -output-directory=/tmp/criew-beamer \
  docs/presentations/kernel-dev-criew-onboarding-beamer.tex
```
