let worker_ready = false;

const form = document.getElementById('inputsForm');
const imageInput = document.getElementById('imageInput');
const varPrefixInput = document.getElementById('varPrefixInput');
const quantizerQualityInput = document.getElementById('quantizerQualityInput');
const submit = document.getElementById('submit-image');

const results = document.getElementById('results');
const downloadZipButton = document.getElementById('downloadZipButton');
const downloadZipLink = document.getElementById('downloadZipLink');

function showProgress(label, percent) {
    const text = document.getElementById('conversionText');
    const progress = document.getElementById('conversionProgress');

    if (percent !== undefined) {
        progress.value = percent;
    } else {
        // Make it indeterminate
        progress.removeAttribute('value');
    }
    text.textContent = label;
}

function updateReadiness() {
    submit.disabled = !worker_ready;
}

form.onsubmit = async (e) => {
    // Browser validation ensures this is okay since we handle submit
    e.preventDefault();

    $('#statusModal').modal({ keyboard: false });
    showProgress('Loading image');

    let file = imageInput.files[0];
    let image_data = await file.arrayBuffer();
    // This slider is expressed as higher-is-better but the underlying
    // quantizer uses lower-is-better so invert it
    let quantizer_quality = (
        Number.parseInt(quantizerQualityInput.max)
        - quantizerQualityInput.valueAsNumber
        + Number.parseInt(quantizerQualityInput.min)
    );
    worker.postMessage({
        image_data: new Uint8Array(image_data),
        image_name: file.name,
        var_prefix: varPrefixInput.value,
        quantizer_quality,
    });
    console.log('Sent', image_data.byteLength, 'bytes to process');
};

const preview = document.getElementById('preview');
preview.onload = () => {
    URL.revokeObjectURL(preview.src);
};

function imageChanged() {
    updateReadiness();
    if (imageInput.files.length == 0) {
        preview.src = null;
    } else {
        preview.src = URL.createObjectURL(imageInput.files[0]);
    }
}
imageInput.onchange = imageChanged;

function showDragActive(active) {
    let f;
    if (active) {
        f = imageInput.classList.add("bg-info");
    } else {
        f = imageInput.classList.remove("bg-info");
    }
}
document.ondragenter = e => {
    if (e.dataTransfer.files) {
        e.dataTransfer.dropEffect = "copy";
        showDragActive(true);
    }
};
document.ondragover = e => {
    e.preventDefault();
};
document.ondrop = e => {
    e.preventDefault();
    showDragActive(false);
    if (e.dataTransfer.files) {
        imageInput.files = e.dataTransfer.files;
        imageChanged();
    }
};
document.ondragend = document.ondragexit = e => {
    showDragActive(false);
};

const worker = new Worker('worker.js');
worker.onmessage = (e) => {
    worker_ready = !!e.data.ready;
    updateReadiness();

    console.log('Received message from worker', e.data);
    if (e.data.error) {
        results.replaceChildren(
            document.createTextNode('Failed: ' + e.data.error),
        );
    } else if (e.data.progress) {
        const {kind, percent} = e.data.progress;
        showProgress(kind, percent);
    } else if (e.data.zip) {
        const {data, name} = e.data.zip;

        $(downloadZipButton).one('click', () => {
            downloadZipLink.href = URL.createObjectURL(new Blob([data]));
            downloadZipLink.download = name + '.zip'
            downloadZipLink.click();
            // click won't run synchronously, so let the event loop turn then
            // we can safely revoke the data URL
            setTimeout(() => {
                downloadZipButton.disabled = true;
                URL.revokeObjectURL(downloadZipLink.href);
            }, 0);
        });
        downloadZipButton.disabled = false;
        showProgress('Conversion complete!', 1);
    }
};
