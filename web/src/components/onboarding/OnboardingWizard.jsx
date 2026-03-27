import { useState, useCallback, useRef, useEffect } from 'react'
import { StarpodIcon } from '../ui/Logo'
import StepIdentity from './StepIdentity'
import StepModel from './StepModel'
import StepRole from './StepRole'
import StepSkills from './StepSkills'
import StepDone from './StepDone'

const STEPS = ['Identity', 'Model', 'Role', 'Skills']

export default function OnboardingWizard({ initialData, onComplete }) {
  const [phase, setPhase] = useState('intro') // intro | steps | done
  const [step, setStep] = useState(0)
  const [transitioning, setTransitioning] = useState(false)
  const [data, setData] = useState({
    agentName: initialData?.agent_name || 'Nova',
    provider: initialData?.provider || 'anthropic',
    model: '',
    generatedSkills: [],
  })

  const updateData = useCallback((updates) => {
    setData(prev => ({ ...prev, ...updates }))
  }, [])

  const animateTransition = useCallback((newStep) => {
    setTransitioning(true)
    setTimeout(() => {
      if (newStep >= STEPS.length) {
        setPhase('done')
      } else {
        setStep(newStep)
      }
      setTransitioning(false)
    }, 150)
  }, [])

  const next = useCallback(() => animateTransition(step + 1), [step, animateTransition])
  const back = useCallback(() => animateTransition(step - 1), [step, animateTransition])

  if (phase === 'intro') {
    return <IntroScreen onStart={() => setPhase('steps')} />
  }

  if (phase === 'done') {
    return <StepDone agentName={data.agentName} onComplete={onComplete} />
  }

  const stepComponent = (() => {
    switch (step) {
      case 0: return <StepIdentity data={data} updateData={updateData} onNext={next} />
      case 1: return <StepModel data={data} updateData={updateData} onNext={next} onBack={back} />
      case 2: return <StepRole data={data} updateData={updateData} onNext={next} onBack={back} />
      case 3: return <StepSkills data={data} onNext={next} onBack={back} />
      default: return null
    }
  })()

  return (
    <div className="ob-shell">
      {/* Progress — segmented bar with step labels */}
      <div className="ob-header">
        <div className="ob-progress">
          {STEPS.map((label, i) => (
            <div key={i} className="ob-progress-step">
              <div
                className={`ob-progress-bar ${i <= step ? 'ob-progress-bar--filled' : ''}`}
              />
              <span className={`ob-progress-label ${i <= step ? 'ob-progress-label--active' : ''}`}>
                {label}
              </span>
            </div>
          ))}
        </div>
      </div>

      {/* Content */}
      <div className="ob-content-area">
        <div className={`ob-step-container ${transitioning ? 'ob-fading' : ''}`}>
          {stepComponent}
        </div>
      </div>
    </div>
  )
}

function IntroScreen({ onStart }) {
  const [visible, setVisible] = useState(false)

  useEffect(() => {
    const t = setTimeout(() => setVisible(true), 50)
    return () => clearTimeout(t)
  }, [])

  return (
    <div className={`ob-intro ${visible ? 'ob-intro--visible' : ''}`}>
      <div className="ob-intro-content">
        <StarpodIcon className="w-12 h-12 text-primary" />
        <h1 className="ob-intro-brand">Starpod</h1>
        <p className="ob-intro-sub">Set up your agent</p>
        <button className="ob-intro-btn" onClick={onStart} autoFocus>
          Begin
        </button>
      </div>
    </div>
  )
}
