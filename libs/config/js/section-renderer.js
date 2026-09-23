import { createActionBadges } from './action-metadata.js';

export function createTopLevelSectionBlock(title, description = '', actions = [], fieldCount = 0) {
    const root = document.createElement('section');
    root.className = fieldCount > 0 && fieldCount <= 2 ? 'config-block config-block-compact' : 'config-block';

    const header = document.createElement('header');
    header.className = 'config-block-header';
    header.appendChild(createHeading(title));
    const meta = createSectionMeta(description, actions);
    if (meta) {
        header.appendChild(meta);
    }

    const section = document.createElement('div');
    section.className = 'config-section';
    root.append(header, section);

    return { root, section };
}

function createHeading(title) {
    const heading = document.createElement('h2');
    heading.className = headingClassName(title);
    heading.textContent = title;
    heading.title = title;
    return heading;
}

function createSectionMeta(description, actions) {
    if (!description && actions.length === 0) {
        return null;
    }

    const meta = document.createElement('div');
    meta.className = 'config-block-meta';

    if (description) {
        meta.appendChild(createSectionDescription(description));
    }
    if (actions.length > 0) {
        meta.appendChild(createActionBadges(actions));
    }

    return meta;
}

function createSectionDescription(description) {
    const text = document.createElement('p');
    text.className = 'section-copy';
    text.textContent = description;
    return text;
}

function headingClassName(text) {
    if (text.length > 28) return 'section-title-tight';
    if (text.length > 20) return 'section-title-compact';
    return '';
}
