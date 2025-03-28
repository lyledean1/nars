# nars

**Na**no in **R**u**s**t - NARS - a terminal with LLM code predictions powered by Ollama. *Work in progress*.  

## Install 

```
cargo install nars
```

## Ollama Code Predictons 

You will need [Ollama](https://ollama.com/) running for code predictions, download and install this before running `nars`.

## Running 

```
nars {filename}
```

And in another terminal, run ollama 

```
ollama run qwen2.5-coder:7b     
```

You will then be able to edit the file. Some key commands:
- Double tap `tab` to stream predictions from Ollama
- "ctrl" + "s" to save 
- `esc` to exit

## Models

The default is currently `qwen2.5-coder:7b`, you can configure this as the second input to nars
```
nars {filename} {model}
```

You will also need to run Ollama with the accompanying model.

## Running Locally

You'll need to make sure after cloning the repo, you sync the submodules which have a number of tree sitter's. 

Once cloned, run 
```
git submodule init && git submodule update
```
Or run, before cloning 
```
git clone --recurse-submodules https://github.com/lyledean1/nars
```
