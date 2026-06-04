function wireNativeClassPrototypes(binding) {
  const metadata = binding.__napiClassMetadata
  if (!Array.isArray(metadata)) {
    return
  }

  const classKey = (item) => `${typeof item.module === 'string' ? item.module : ''}\0${item.name}`
  const exportedClassValue = (item) => {
    const target =
      typeof item.module === 'string' && item.module.length > 0
        ? binding[item.module]
        : binding
    return target == null ? undefined : target[item.name]
  }

  const classes = new Map()
  for (const item of metadata) {
    if (item && typeof item.name === 'string' && typeof item.constructor === 'function') {
      classes.set(classKey(item), item)
    }
  }

  for (const item of classes.values()) {
    if (!item.parent) {
      continue
    }

    const parent = classes.get(classKey({ module: item.module, name: item.parent }))
    if (!parent) {
      throw new Error(`Native class parent ${item.parent} for ${item.name} is not registered`)
    }

    Object.setPrototypeOf(item.constructor.prototype, parent.constructor.prototype)
    if (Object.getPrototypeOf(item.constructor.prototype) !== parent.constructor.prototype) {
      throw new Error(`Failed to wire native class prototype for ${item.name}`)
    }

    if (item.constructible && parent.constructible) {
      const childExport = exportedClassValue(item)
      const parentExport = exportedClassValue(parent)
      if (typeof childExport !== 'function' || typeof parentExport !== 'function') {
        throw new Error(`Native class constructor edge for ${item.name} is not exported as functions`)
      }

      Object.setPrototypeOf(childExport, parentExport)
      if (Object.getPrototypeOf(childExport) !== parentExport) {
        throw new Error(`Failed to wire native class constructor for ${item.name}`)
      }
    }
  }

  const IteratorCtor = globalThis.Iterator
  if (typeof IteratorCtor === 'function' && IteratorCtor.prototype) {
    for (const item of classes.values()) {
      if (!item.iterator || item.parent) {
        continue
      }

      Object.setPrototypeOf(item.constructor.prototype, IteratorCtor.prototype)
      if (Object.getPrototypeOf(item.constructor.prototype) !== IteratorCtor.prototype) {
        throw new Error(`Failed to wire native iterator prototype for ${item.name}`)
      }
    }
  }
}

function hideNativeClassMetadata(binding) {
  if (Object.prototype.hasOwnProperty.call(binding, '__napiClassMetadata')) {
    delete binding.__napiClassMetadata
  }
}
